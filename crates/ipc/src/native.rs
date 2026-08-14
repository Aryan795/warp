//! This module implements IPC transport on top of the `interprocess` crate, which uses Unix Domain
//! Sockets on Unix platforms and named pipes on Windows under the hood.
use async_compat::CompatExt as _;
use futures::{AsyncRead, AsyncWrite};

use crate::ConnectionAddress;

pub(crate) mod client {
    use interprocess::local_socket::tokio::LocalSocketStream;

    use super::*;
    use crate::client::{ClientError, InitializationError, Result};

    /// Returns a tuple containing structs for reading and writing to a local socket, which is the
    /// underlying IPC transport for native (non-wasm) platforms.
    pub async fn connect_client(
        connection_address: ConnectionAddress,
    ) -> Result<(impl AsyncRead + Unpin, impl AsyncWrite + Unpin)> {
        let stream = LocalSocketStream::connect(connection_address.0.as_str())
            .compat()
            .await
            .map_err(|e| ClientError::Initialization(InitializationError::Io(e)))?;
        Ok(stream.into_split())
    }
}

pub(crate) mod server {
    use interprocess::local_socket::tokio::{LocalSocketListener, LocalSocketStream};

    use super::*;
    use crate::server::{InitializationError, Result, ServerError};

    /// Server-side connection stream. On Windows, this may be backed by either the
    /// `interprocess`-managed transport or, when a security descriptor is requested (see
    /// [`ConnectionListenerImpl::new`]), by a named pipe created directly via
    /// [`windows_pipe`] since `interprocess` does not expose a way to customize named pipe
    /// security attributes.
    enum ConnectionStream {
        Standard(LocalSocketStream),
        #[cfg(windows)]
        WindowsPipe(windows_pipe::PipeStream),
    }

    pub struct ConnectionImpl {
        stream: ConnectionStream,
    }

    impl ConnectionImpl {
        pub fn into_split(
            self,
        ) -> (
            Box<dyn AsyncRead + Send + Unpin>,
            Box<dyn AsyncWrite + Send + Unpin>,
        ) {
            match self.stream {
                ConnectionStream::Standard(stream) => {
                    let (reader, writer) = stream.into_split();
                    (Box::new(reader), Box::new(writer))
                }
                #[cfg(windows)]
                ConnectionStream::WindowsPipe(stream) => {
                    let (reader, writer) = stream.into_split();
                    (Box::new(reader), Box::new(writer))
                }
            }
        }
    }

    enum ListenerImpl {
        Standard(LocalSocketListener),
        #[cfg(windows)]
        WindowsPipe(windows_pipe::PipeListener),
    }

    pub struct ConnectionListenerImpl {
        listener: ListenerImpl,
    }

    impl ConnectionListenerImpl {
        /// Creates a listener for `connection_address`.
        ///
        /// `windows_pipe_security_descriptor`, when set, requests that the underlying named pipe
        /// be created with the given SDDL security descriptor instead of the OS default. This is
        /// ignored outside of Windows, where local sockets are Unix Domain Sockets rather than
        /// named pipes and thus have no equivalent concept of a security descriptor.
        pub fn new(
            connection_address: ConnectionAddress,
            windows_pipe_security_descriptor: Option<&str>,
        ) -> Result<Self> {
            #[cfg(windows)]
            if let Some(sddl) = windows_pipe_security_descriptor {
                let listener = windows_pipe::PipeListener::bind(&connection_address, sddl)
                    .map_err(|e| ServerError::Initialization(InitializationError::Io(e)))?;
                return Ok(Self {
                    listener: ListenerImpl::WindowsPipe(listener),
                });
            }
            #[cfg(not(windows))]
            let _ = windows_pipe_security_descriptor;

            let listener = warpui_core::r#async::block_on(
                async move { LocalSocketListener::bind(connection_address.to_string()) }.compat(),
            )
            .map_err(|e| ServerError::Initialization(InitializationError::Io(e)))?;
            Ok(Self {
                listener: ListenerImpl::Standard(listener),
            })
        }

        pub async fn accept_connection(&self) -> Result<ConnectionImpl> {
            match &self.listener {
                ListenerImpl::Standard(listener) => listener
                    .accept()
                    .compat()
                    .await
                    .map(|stream| ConnectionImpl {
                        stream: ConnectionStream::Standard(stream),
                    })
                    .map_err(ServerError::AcceptConnection),
                #[cfg(windows)]
                ListenerImpl::WindowsPipe(listener) => listener
                    .accept()
                    .await
                    .map(|stream| ConnectionImpl {
                        stream: ConnectionStream::WindowsPipe(stream),
                    })
                    .map_err(ServerError::AcceptConnection),
            }
        }
    }

    /// Windows named pipe transport that bypasses `interprocess`'s pipe creation so that an
    /// explicit security descriptor can be attached. `interprocess` (as of 1.2.1) always creates
    /// named pipes with `lpSecurityAttributes = NULL`, which grants the default security
    /// descriptor (full control to the creator, read-only to Everyone). That default is
    /// insufficient for servers that must accept connections from a client running at a
    /// different elevation level than the server (see the single-instance URI channel in
    /// `app_services::windows`, which is the sole caller of this path).
    #[cfg(windows)]
    mod windows_pipe {
        use std::ffi::c_void;
        use std::io;

        use tokio::net::windows::named_pipe::{self, NamedPipeServer};
        use tokio::sync::Mutex;
        use tokio_util::compat::{
            Compat, TokioAsyncReadCompatExt as _, TokioAsyncWriteCompatExt as _,
        };
        use windows::Win32::Foundation::{HLOCAL, LocalFree};
        use windows::Win32::Security::Authorization::{
            ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
        };
        use windows::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
        use windows::core::PCWSTR;

        use crate::ConnectionAddress;

        /// RAII wrapper around a security descriptor allocated by
        /// `ConvertStringSecurityDescriptorToSecurityDescriptorW`, which the caller is
        /// responsible for freeing with `LocalFree`.
        struct OwnedSecurityDescriptor(PSECURITY_DESCRIPTOR);

        // SAFETY: the security descriptor is only ever read (not mutated) after construction, so
        // it's safe to share a reference to it across threads/tasks.
        unsafe impl Send for OwnedSecurityDescriptor {}
        unsafe impl Sync for OwnedSecurityDescriptor {}

        impl Drop for OwnedSecurityDescriptor {
            fn drop(&mut self) {
                if !self.0.0.is_null() {
                    unsafe {
                        let _ = LocalFree(Some(HLOCAL(self.0.0)));
                    }
                }
            }
        }

        /// Parses `sddl` into a Windows security descriptor.
        fn parse_security_descriptor(sddl: &str) -> io::Result<OwnedSecurityDescriptor> {
            let sddl_wide: Vec<u16> = sddl.encode_utf16().chain(std::iter::once(0)).collect();
            let mut descriptor = PSECURITY_DESCRIPTOR::default();
            unsafe {
                ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    PCWSTR(sddl_wide.as_ptr()),
                    SDDL_REVISION_1,
                    &mut descriptor,
                    None,
                )
            }
            .map_err(|e| io::Error::other(format!("Failed to parse SDDL {sddl:?}: {e:#}")))?;
            Ok(OwnedSecurityDescriptor(descriptor))
        }

        /// Returns the full `\\.\pipe\<name>` path for `connection_address`, matching the path
        /// `interprocess` derives internally for the same connection address so that clients
        /// connecting via `interprocess` (see `client::connect_client`) can still reach this pipe.
        fn pipe_path(connection_address: &ConnectionAddress) -> String {
            format!(r"\\.\pipe\{connection_address}")
        }

        pub struct PipeStream(NamedPipeServer);

        impl PipeStream {
            pub fn into_split(
                self,
            ) -> (
                Compat<tokio::io::ReadHalf<NamedPipeServer>>,
                Compat<tokio::io::WriteHalf<NamedPipeServer>>,
            ) {
                let (reader, writer) = tokio::io::split(self.0);
                (reader.compat(), writer.compat_write())
            }
        }

        pub struct PipeListener {
            path: String,
            security_descriptor: OwnedSecurityDescriptor,
            // Per Windows' named pipe semantics, only the very first instance created for a given
            // pipe name establishes its security descriptor; subsequent instances (created here
            // after each `accept`) ignore whatever security attributes they're given. We keep
            // building fresh `SECURITY_ATTRIBUTES` from the same descriptor for every instance
            // anyway, since `CreateNamedPipeW` requires *some* value to be supplied.
            stored_instance: Mutex<NamedPipeServer>,
        }

        impl PipeListener {
            pub fn bind(connection_address: &ConnectionAddress, sddl: &str) -> io::Result<Self> {
                let path = pipe_path(connection_address);
                let security_descriptor = parse_security_descriptor(sddl)?;
                // `first_pipe_instance` guards against "named pipe squatting": without it, a
                // malicious process could pre-create a pipe with this name before we do, and we'd
                // silently become an additional instance of that attacker-controlled pipe instead
                // of failing loudly.
                let first_instance = Self::create_pipe_instance(&path, &security_descriptor, true)?;
                Ok(Self {
                    path,
                    security_descriptor,
                    stored_instance: Mutex::new(first_instance),
                })
            }

            fn create_pipe_instance(
                path: &str,
                security_descriptor: &OwnedSecurityDescriptor,
                first_instance: bool,
            ) -> io::Result<NamedPipeServer> {
                let mut attributes = SECURITY_ATTRIBUTES {
                    nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                    lpSecurityDescriptor: security_descriptor.0.0,
                    bInheritHandle: false.into(),
                };
                // SAFETY: `attributes` is a validly-initialized `SECURITY_ATTRIBUTES` whose
                // `lpSecurityDescriptor` points at a security descriptor that outlives this call
                // (owned by `self` for the lifetime of the listener). The OS only reads through
                // this pointer for the duration of the `CreateNamedPipeW` call underlying
                // `create_with_security_attributes_raw`.
                unsafe {
                    named_pipe::ServerOptions::new()
                        .first_pipe_instance(first_instance)
                        .create_with_security_attributes_raw(
                            path,
                            &mut attributes as *mut SECURITY_ATTRIBUTES as *mut c_void,
                        )
                }
            }

            pub async fn accept(&self) -> io::Result<PipeStream> {
                let mut stored_instance = self.stored_instance.lock().await;
                stored_instance.connect().await?;
                let next_instance =
                    Self::create_pipe_instance(&self.path, &self.security_descriptor, false)?;
                let connected_instance = std::mem::replace(&mut *stored_instance, next_instance);
                Ok(PipeStream(connected_instance))
            }
        }
    }
}
