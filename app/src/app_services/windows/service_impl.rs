use std::sync::Arc;
use std::time::Duration;

use async_channel::Sender;
use async_trait::async_trait;
use ipc::{Client, ClientError, ConnectionAddress};
use url::Url;
use warp_errors::report_error;
use warpui::r#async::Timer;
use warpui::r#async::executor::Background;
use windows::Win32::UI::WindowsAndMessaging::{ASFW_ANY, AllowSetForegroundWindow};

use super::single_instance_manager::uri_named_pipe_name;

/// How long to keep trying to reach the existing instance's URI pipe before concluding that there
/// is no reachable instance.
///
/// The claim on the single-instance mutex and the pipe that serves it are established together, but
/// not atomically: a launch can test the mutex in the window between the two, and a listener can
/// momentarily have no free connection instance. Both resolve in well under a millisecond, so the
/// budget only has to be long enough to ride them out and short enough that a launch which really
/// has nobody to talk to still starts promptly.
const CONNECT_RETRY_BUDGET: Duration = Duration::from_millis(750);

/// How long to wait between connection attempts within [`CONNECT_RETRY_BUDGET`].
const CONNECT_RETRY_INTERVAL: Duration = Duration::from_millis(25);

/// IPC Service to respond to URIs sent to the active Warp instance.
pub(super) struct UriService {}

impl ipc::Service for UriService {
    type Request = Vec<Url>;
    type Response = ();
}

#[derive(Clone)]
pub(super) struct UriServiceImpl {
    tx: Sender<Vec<Url>>,
}

impl UriServiceImpl {
    pub(super) fn new(tx: Sender<Vec<Url>>) -> Self {
        Self { tx }
    }
}

#[async_trait]
impl ipc::ServiceImpl for UriServiceImpl {
    type Service = UriService;

    async fn handle_request(&self, request: Vec<Url>) -> () {
        if let Err(send_error) = self.tx.send(request).await {
            report_error!(
                anyhow::Error::new(send_error).context("Error sending urls to local stream")
            );
        }
    }
}

/// Lets the existing instance pull itself to the foreground when it handles the hand-off.
///
/// Windows refuses `SetForegroundWindow` from a process that is not already in the foreground and
/// only flashes its taskbar button instead, so without this grant a redirected launch is
/// indistinguishable from one that did nothing - and the user launches Warp again. This process was
/// started by whatever the user just interacted with (Explorer, a shell, a browser), so it does
/// hold the right and can pass it on. It exits immediately afterwards, and Windows revokes the
/// grant on the next user input.
fn allow_existing_instance_to_take_foreground() {
    // SAFETY: no pointer or handle arguments to keep valid; the call only adjusts which processes
    // may take the foreground.
    if let Err(err) = unsafe { AllowSetForegroundWindow(ASFW_ANY) } {
        log::warn!("Failed to grant foreground rights to the existing Warp instance: {err}");
    }
}

/// Connects to the existing instance's URI pipe, retrying transient failures within
/// [`CONNECT_RETRY_BUDGET`].
async fn connect_to_sole_running_instance(
    background_executor: Arc<Background>,
) -> Result<Client, ClientError> {
    let mut remaining_budget = CONNECT_RETRY_BUDGET;
    loop {
        match Client::connect(
            ConnectionAddress::from(uri_named_pipe_name()),
            background_executor.clone(),
        )
        .await
        {
            Ok(client) => return Ok(client),
            Err(err) => {
                if !err.is_transient_connect_failure() || remaining_budget.is_zero() {
                    return Err(err);
                }
                log::debug!("Retrying connection to the existing Warp instance: {err}");
                let interval = CONNECT_RETRY_INTERVAL.min(remaining_budget);
                Timer::after(interval).await;
                remaining_budget -= interval;
            }
        }
    }
}

/// Forwards the given URLs to the main running instance of Warp.
pub(super) async fn forward_uri_to_sole_running_instance(
    urls: Vec<Url>,
) -> Result<(), ClientError> {
    // We need to construct a new background executor because this function is
    // run before we have a `AppContext`.  We explicitly create it with
    // a single backing thread, as we don't need an entire pool of threads.
    let background_executor = Arc::new(Background::new(1, |_| "forward-uris".to_owned()));
    let client = connect_to_sole_running_instance(background_executor).await?;
    allow_existing_instance_to_take_foreground();
    let uri_service_caller = ipc::service_caller::<UriService>(Arc::new(client));
    uri_service_caller.call(urls).await?;
    Ok(())
}
