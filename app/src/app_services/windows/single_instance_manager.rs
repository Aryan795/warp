use std::sync::{Arc, LazyLock};

use async_channel::Receiver;
use ipc::ServerBuilder;
use parking_lot::Mutex;
use url::Url;
use warp_core::channel::ChannelState;
use warp_errors::report_error;
use warpui::r#async::executor::Background;
use warpui::{Entity, ModelContext, SingletonEntity};
use windows::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE};
use windows::Win32::System::Threading::CreateMutexW;
use windows::core::Error;

use super::service_impl::UriServiceImpl;

/// RAII wrapper around a Windows mutex HANDLE that closes it on drop.
struct MutexHandle(HANDLE);

// SAFETY: Windows kernel mutexes are valid to use from any thread. For example it says here:
// https://learn.microsoft.com/en-us/windows/win32/api/synchapi/nf-synchapi-createmutexw#remarks
// > "Any thread of the calling process can specify the mutex-object handle in a call to one of the
//   wait functions"
// The [`HANDLE`] is not Send or Sync b/c it's a common type used to point to a variety of Windows
// kernel objects, many of which are not safe to access from other threads.
unsafe impl Send for MutexHandle {}
unsafe impl Sync for MutexHandle {}

impl Drop for MutexHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

/// The role this process plays in single-instance enforcement. Resolved on first access and fixed
/// for the process lifetime.
///
/// It's a complex type. Breaking it down:
/// * LazyLock - This type lets us go from un-initialized to initialized without `mut` and _not_
///   vice-versa.
/// * Mutex - Gives us interior mutability. Unlike `RefCell` it can be used in statics since it is
///   Sync. We don't actually need to access it on other threads though.
/// * Result - CreateMutexW might fail for reasons other than another process holding the lock. In
///   those cases, we store the error type.
static INSTANCE_ROLE: LazyLock<Mutex<Result<InstanceRole, Error>>> =
    LazyLock::new(|| Mutex::new(claim_sole_instance()));

enum InstanceRole {
    /// This process owns the single-instance claim and is listening for hand-offs from later
    /// launches.
    Sole(SoleInstance),
    /// Another process owns the claim, so this launch should hand its startup arguments over
    /// instead of starting a second GUI.
    Secondary,
    /// This process could not listen for hand-offs, so it gave the claim back rather than hold one
    /// that nothing can reach. It runs as a full instance, but later launches will not find it and
    /// will start their own.
    Undiscoverable,
}

/// The resources that make this process the sole instance. Each is held for the process lifetime;
/// dropping any of them gives up part of the claim.
struct SoleInstance {
    /// The named kernel mutex that other launches test for. Released by the OS once every handle
    /// to it is closed, including on a crash.
    _mutex: MutexHandle,
    /// The listening end of the URI named pipe.
    _server: ipc::Server,
    /// Drives the server's accept loop. The claim is taken long before there is an `AppContext` to
    /// borrow an executor from, so the claim owns one.
    _executor: Arc<Background>,
    /// Hand-offs received on the pipe, buffered rather than dropped: the claim is visible to other
    /// launches from the moment it is taken, but this process cannot act on a URI until it has
    /// finished initializing.
    forwarded_uris: Receiver<Vec<Url>>,
}

pub(super) fn uri_named_pipe_name() -> String {
    format!("Warp{:?}_URI_CHANNEL", ChannelState::channel())
}

/// The name of the single-instance mutex.
///
/// NOTE: This name must stay in sync with `AppMutexName` in
/// `script/windows/windows-installer.iss`, which the installer uses to detect whether Warp is
/// running.
fn single_instance_mutex_name() -> String {
    // Scope this lock to the specific user session.
    // https://learn.microsoft.com/en-us/windows/win32/termserv/kernel-object-namespaces
    // > "client processes can use the "Local\" prefix to explicitly create an object in their
    //   session namespace"
    format!("Local\\Warp{:?}_SingleInstance", ChannelState::channel())
}

/// Creates the single-instance mutex. Returns `None` when another process already owns it.
fn try_create_mutex() -> Result<Option<MutexHandle>, Error> {
    let name = single_instance_mutex_name()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect::<Vec<u16>>();
    let handle = unsafe { CreateMutexW(None, true, windows::core::PCWSTR(name.as_ptr())) };

    // https://learn.microsoft.com/en-us/windows/win32/api/synchapi/nf-synchapi-createmutexw#return-value
    let already_exists = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
    handle
        .inspect_err(|err| {
            report_error!(
                anyhow::Error::new(err.clone()).context("Failed to create single-instance mutex")
            );
        })
        .map(|handle| {
            if already_exists {
                // Another instance already owns this mutex. Close our duplicate handle.
                unsafe {
                    let _ = CloseHandle(handle);
                }
                None
            } else {
                Some(MutexHandle(handle))
            }
        })
}

/// Claims the single-instance role for this process.
///
/// The mutex and the URI pipe are established in one step, and the pipe is accepting connections
/// before this function returns: `ServerBuilder::build_and_run` creates the pipe synchronously and
/// only the accept loop runs on the executor. That ordering is the whole point. The mutex is what
/// later launches test to decide whether to hand their startup arguments over, so a process that
/// holds the mutex without listening strands every launch that finds it - which is what happened
/// while the pipe was created later, during GUI initialization.
///
/// If the pipe cannot be created, the mutex is released rather than held by a process that no
/// launch can reach.
fn claim_sole_instance() -> Result<InstanceRole, Error> {
    let Some(mutex) = try_create_mutex()? else {
        return Ok(InstanceRole::Secondary);
    };

    let executor = Arc::new(Background::new(1, |_| "uri-server".to_owned()));
    let (tx, forwarded_uris) = async_channel::unbounded();
    match ServerBuilder::default()
        .with_fixed_address(uri_named_pipe_name())
        .with_service(UriServiceImpl::new(tx))
        .build_and_run(executor.clone())
    {
        Ok((server, _)) => Ok(InstanceRole::Sole(SoleInstance {
            _mutex: mutex,
            _server: server,
            _executor: executor,
            forwarded_uris,
        })),
        Err(err) => {
            report_error!(
                anyhow::Error::new(err).context("Failed to initialize UriService Server")
            );
            drop(mutex);
            Ok(InstanceRole::Undiscoverable)
        }
    }
}

/// A singleton model that is responsible for ensuring there is only one instance of Warp running.
/// Uses a Windows named mutex (via `CreateMutexW`) which is a kernel object automatically cleaned
/// up by the OS when all handles are closed, including on crash.
pub(super) struct SingleInstanceManager {}

impl SingleInstanceManager {
    /// Starts handling the URIs that later launches have handed to this process.
    ///
    /// Hand-offs that arrived before this point were buffered when the claim was taken, so a
    /// launch redirected here during startup is answered once the app can act on it instead of
    /// being lost.
    pub(super) fn new(ctx: &mut ModelContext<Self>) -> Self {
        let forwarded_uris = match &*INSTANCE_ROLE.lock() {
            Ok(InstanceRole::Sole(sole_instance)) => sole_instance.forwarded_uris.clone(),
            Ok(InstanceRole::Secondary | InstanceRole::Undiscoverable) | Err(_) => {
                return Self {};
            }
        };

        ctx.spawn_stream_local(
            forwarded_uris,
            |_single_instance_manager, uris, ctx| {
                for uri in uris {
                    crate::uri::handle_incoming_uri(&uri, ctx);
                }
            },
            |_, _| {},
        );

        Self {}
    }

    /// Whether another instance of Warp holds the single-instance claim, meaning this launch should
    /// hand its startup arguments to that instance instead of starting a second GUI.
    ///
    /// `Undiscoverable` reports `false`: that process is not the sole instance in any meaningful
    /// sense, but there is no other instance to hand over to either, so it has to launch.
    pub(super) fn has_existing_instance() -> Result<bool, Error> {
        match &*INSTANCE_ROLE.lock() {
            Ok(InstanceRole::Secondary) => Ok(true),
            Ok(InstanceRole::Sole(_) | InstanceRole::Undiscoverable) => Ok(false),
            Err(err) => Err(err.clone()),
        }
    }
}

impl Entity for SingleInstanceManager {
    type Event = ();
}

impl SingletonEntity for SingleInstanceManager {}
