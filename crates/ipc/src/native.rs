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

    pub struct ConnectionImpl {
        stream: LocalSocketStream,
    }

    impl ConnectionImpl {
        pub fn into_split(self) -> (impl AsyncRead + Unpin, impl AsyncWrite + Unpin) {
            self.stream.into_split()
        }
    }

    pub struct ConnectionListenerImpl {
        listener: LocalSocketListener,
    }

    impl ConnectionListenerImpl {
        pub fn new(connection_address: ConnectionAddress) -> Result<Self> {
            // Cloned up-front since `connection_address` is moved into the `bind` future below,
            // but we still need it afterwards on Windows to locate the pipe for DACL hardening.
            #[cfg(windows)]
            let connection_address_for_dacl = connection_address.clone();

            let listener = warpui_core::r#async::block_on(
                async move { LocalSocketListener::bind(connection_address.to_string()) }.compat(),
            )
            .map_err(|e| ServerError::Initialization(InitializationError::Io(e)))?;

            // On Windows, the pipe `interprocess` just created keeps the OS default DACL, which
            // is the root cause of REV-1546 (cross-elevation `ERROR_ACCESS_DENIED` on the
            // single-instance URI pipe). Restrict it to the current user (across elevation
            // levels), `SYSTEM`, and `Administrators`.
            #[cfg(windows)]
            harden_named_pipe_dacl(&connection_address_for_dacl);

            Ok(Self { listener })
        }

        pub async fn accept_connection(&self) -> Result<ConnectionImpl> {
            self.listener
                .accept()
                .compat()
                .await
                .map(|stream| ConnectionImpl { stream })
                .map_err(ServerError::AcceptConnection)
        }
    }

    /// Builds the full Windows named-pipe path (`\\.\pipe\<name>`) for the given local socket
    /// name. `interprocess`'s `LocalSocketListener` performs the same transformation internally
    /// when given a plain (non-namespaced) name, so this must be kept in sync with it for
    /// `restrict_named_pipe_to_current_user` to target the right pipe object.
    #[allow(dead_code)]
    pub(crate) fn windows_named_pipe_path(name: &str) -> String {
        format!(r"\\.\pipe\{name}")
    }

    /// Restricts the DACL on the just-created named pipe so that a later, differently-elevated
    /// process for the same user can still connect to it. See `crate::windows_pipe_security` for
    /// the full rationale (REV-1546). Failures are logged and otherwise ignored -- the pipe keeps
    /// functioning under Windows' default DACL, which is what every prior release of Warp already
    /// relied on.
    #[cfg(windows)]
    fn harden_named_pipe_dacl(connection_address: &ConnectionAddress) {
        let pipe_path = windows_named_pipe_path(&connection_address.to_string());
        if let Err(err) =
            crate::windows_pipe_security::restrict_named_pipe_to_current_user(&pipe_path)
        {
            log::warn!(
                "Failed to restrict ACL on named pipe {pipe_path}; it will keep the OS default \
                 DACL, which can cause cross-elevation IPC failures (REV-1546): {err:?}"
            );
        }
    }
}

#[cfg(test)]
#[path = "native_server_tests.rs"]
mod native_server_tests;
