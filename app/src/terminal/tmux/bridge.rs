use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

use parking_lot::{FairMutex, Mutex};
use warpui::WindowId;

use super::parser::PaneId;
use crate::terminal::TerminalModel;
use crate::terminal::model::ansi;
use crate::terminal::model::terminal_model::TmuxClientEvent;
use crate::terminal::shell::ShellType;
use crate::terminal::tmux::pane_bytes::sink_writer;

const MAX_BUFFERED_PANE_BYTES: usize = 64 * 1024;
const MAX_UNREGISTERED_PANE_COUNT: usize = 64;
const MAX_UNREGISTERED_PANE_BYTES: usize = 256 * 1024;
const MAX_PENDING_CLIENT_EVENT_BYTES: usize = 8 * 1024;
const PENDING_CLIENT_EVENT_OVERHEAD: usize = 64;
pub const APP_BIND_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(8);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TmuxInstanceId(u64);

impl TmuxInstanceId {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }

    pub fn as_u64(self) -> u64 {
        self.0
    }

    pub fn from_u64(id: u64) -> Self {
        Self(id)
    }
}

struct PaneSink {
    model: Arc<FairMutex<TerminalModel>>,
    processor: ansi::Processor,
}

struct Inner {
    gateway_window: Option<WindowId>,
    presentation_window: Option<WindowId>,
    panes: HashMap<String, PaneSink>,
    buffers: HashMap<String, Vec<u8>>,
    pending_captures: VecDeque<String>,
    pending_client_events: Vec<TmuxClientEvent>,
    pending_client_event_bytes: usize,
    bootstrapped_panes: HashSet<String>,
    pending_bootstrap_panes: VecDeque<String>,
    shell_type: Option<ShellType>,
    app_bind_deadline: Option<instant::Instant>,
    presentation_ready: bool,
    unregistered_bytes: usize,
}

/// One Warp-managed tmux control-mode session (gateway PTY + presentation window).
pub struct TmuxRuntime {
    id: TmuxInstanceId,
    inner: Mutex<Inner>,
    applying: AtomicBool,
}

struct TmuxRuntimeIndex {
    by_id: HashMap<TmuxInstanceId, Arc<TmuxRuntime>>,
    by_gateway: HashMap<WindowId, TmuxInstanceId>,
    by_presentation: HashMap<WindowId, TmuxInstanceId>,
}

fn index() -> parking_lot::MutexGuard<'static, TmuxRuntimeIndex> {
    static INDEX: OnceLock<Mutex<TmuxRuntimeIndex>> = OnceLock::new();
    INDEX
        .get_or_init(|| {
            Mutex::new(TmuxRuntimeIndex {
                by_id: HashMap::new(),
                by_gateway: HashMap::new(),
                by_presentation: HashMap::new(),
            })
        })
        .lock()
}

impl TmuxRuntime {
    pub fn new() -> Arc<Self> {
        let runtime = Arc::new(Self {
            id: TmuxInstanceId::new(),
            inner: Mutex::new(Inner {
                gateway_window: None,
                presentation_window: None,
                panes: HashMap::new(),
                buffers: HashMap::new(),
                pending_captures: VecDeque::new(),
                pending_client_events: Vec::new(),
                pending_client_event_bytes: 0,
                bootstrapped_panes: HashSet::new(),
                pending_bootstrap_panes: VecDeque::new(),
                shell_type: None,
                app_bind_deadline: None,
                presentation_ready: false,
                unregistered_bytes: 0,
            }),
            applying: AtomicBool::new(false),
        });
        index().by_id.insert(runtime.id, runtime.clone());
        runtime
    }

    pub fn id(&self) -> TmuxInstanceId {
        self.id
    }

    pub fn insert(runtime: Arc<Self>) {
        index().by_id.insert(runtime.id, runtime);
    }

    pub fn for_id(id: TmuxInstanceId) -> Option<Arc<Self>> {
        index().by_id.get(&id).cloned()
    }

    pub fn for_gateway(window: WindowId) -> Option<Arc<Self>> {
        let idx = index();
        let id = *idx.by_gateway.get(&window)?;
        idx.by_id.get(&id).cloned()
    }

    pub fn for_presentation(window: WindowId) -> Option<Arc<Self>> {
        let idx = index();
        let id = *idx.by_presentation.get(&window)?;
        idx.by_id.get(&id).cloned()
    }

    pub fn bind_gateway(self: &Arc<Self>, window: WindowId) {
        self.inner.lock().gateway_window = Some(window);
        let mut idx = index();
        idx.by_id.insert(self.id, Arc::clone(self));
        idx.by_gateway.insert(window, self.id);
    }

    pub fn bind_presentation(self: &Arc<Self>, window: WindowId) {
        self.inner.lock().presentation_window = Some(window);
        let mut idx = index();
        idx.by_id.insert(self.id, Arc::clone(self));
        idx.by_presentation.insert(window, self.id);
    }

    pub fn gateway_window(&self) -> Option<WindowId> {
        self.inner.lock().gateway_window
    }

    pub fn presentation_window(&self) -> Option<WindowId> {
        self.inner.lock().presentation_window
    }

    pub fn unregister(&self) {
        let mut inner = self.inner.lock();
        let mut idx = index();
        idx.by_id.remove(&self.id);
        if let Some(gateway) = inner.gateway_window.take() {
            idx.by_gateway.remove(&gateway);
        }
        if let Some(presentation) = inner.presentation_window.take() {
            idx.by_presentation.remove(&presentation);
        }
        self.applying.store(false, Ordering::SeqCst);
        inner.panes.clear();
        inner.buffers.clear();
        inner.pending_captures.clear();
        inner.pending_client_events.clear();
        inner.pending_client_event_bytes = 0;
        inner.bootstrapped_panes.clear();
        inner.pending_bootstrap_panes.clear();
        inner.app_bind_deadline = None;
        inner.presentation_ready = false;
        inner.unregistered_bytes = 0;
    }

    pub fn is_applying(&self) -> bool {
        self.applying.load(Ordering::SeqCst)
    }

    pub fn with_applying<T>(&self, f: impl FnOnce() -> T) -> T {
        self.applying.store(true, Ordering::SeqCst);
        let result = f();
        self.applying.store(false, Ordering::SeqCst);
        result
    }

    pub fn register_pane(&self, pane_id: &str, model: Arc<FairMutex<TerminalModel>>) {
        let mut inner = self.inner.lock();
        let buffered = inner.buffers.remove(pane_id).unwrap_or_default();
        inner.unregistered_bytes = inner.unregistered_bytes.saturating_sub(buffered.len());
        let mut sink = PaneSink {
            model,
            processor: ansi::Processor::new(),
        };
        if !buffered.is_empty() {
            feed_sink(&mut sink, &buffered);
        }
        inner.panes.insert(pane_id.to_owned(), sink);
    }

    pub fn unregister_pane(&self, pane_id: &str) {
        self.inner.lock().panes.remove(pane_id);
    }

    pub fn deliver_output(&self, pane_id: &PaneId, bytes: &[u8]) -> bool {
        let mut inner = self.inner.lock();
        if let Some(sink) = inner.panes.get_mut(pane_id.as_str()) {
            feed_sink(sink, bytes);
            return true;
        }
        let is_new_pane = !inner.buffers.contains_key(pane_id.as_str());
        if is_new_pane && inner.buffers.len() >= MAX_UNREGISTERED_PANE_COUNT {
            clear_unregistered_buffers(&mut inner);
            return false;
        }
        let buffered_len = inner
            .buffers
            .get(pane_id.as_str())
            .map(Vec::len)
            .unwrap_or(0);
        let room = MAX_BUFFERED_PANE_BYTES.saturating_sub(buffered_len);
        if room == 0 {
            return true;
        }
        let take = bytes.len().min(room);
        if inner.unregistered_bytes.saturating_add(take) > MAX_UNREGISTERED_PANE_BYTES {
            clear_unregistered_buffers(&mut inner);
            return false;
        }
        inner
            .buffers
            .entry(pane_id.as_str().to_owned())
            .or_default()
            .extend_from_slice(&bytes[..take]);
        inner.unregistered_bytes += take;
        true
    }

    pub fn note_capture(&self, pane_id: &str) {
        self.inner
            .lock()
            .pending_captures
            .push_back(pane_id.to_owned());
    }

    pub fn take_capture(&self) -> Option<String> {
        self.inner.lock().pending_captures.pop_front()
    }

    pub fn buffer_client_events(&self, events: &[TmuxClientEvent]) -> bool {
        if events.is_empty() {
            return true;
        }
        let mut inner = self.inner.lock();
        for event in events {
            let incoming_bytes = client_event_retained_bytes(event);
            if incoming_bytes > MAX_PENDING_CLIENT_EVENT_BYTES {
                log::warn!(
                    "tmux runtime {} pending client event overflow; rolling back",
                    self.id.as_u64()
                );
                inner.pending_client_events.clear();
                inner.pending_client_event_bytes = 0;
                return false;
            }
            if let TmuxClientEvent::LayoutChange { window_id, .. } = event
                && let Some(TmuxClientEvent::LayoutChange {
                    window_id: last_id, ..
                }) = inner.pending_client_events.last()
                && last_id == window_id
            {
                let previous_bytes =
                    client_event_retained_bytes(inner.pending_client_events.last().unwrap());
                let next_bytes = inner.pending_client_event_bytes - previous_bytes + incoming_bytes;
                if next_bytes > MAX_PENDING_CLIENT_EVENT_BYTES {
                    log::warn!(
                        "tmux runtime {} pending client event overflow; rolling back",
                        self.id.as_u64()
                    );
                    inner.pending_client_events.clear();
                    inner.pending_client_event_bytes = 0;
                    return false;
                }
                *inner.pending_client_events.last_mut().unwrap() = event.clone();
                inner.pending_client_event_bytes = next_bytes;
                continue;
            }
            let next_bytes = inner.pending_client_event_bytes + incoming_bytes;
            if next_bytes > MAX_PENDING_CLIENT_EVENT_BYTES {
                log::warn!(
                    "tmux runtime {} pending client event overflow; rolling back",
                    self.id.as_u64()
                );
                inner.pending_client_events.clear();
                inner.pending_client_event_bytes = 0;
                return false;
            }
            inner.pending_client_events.push(event.clone());
            inner.pending_client_event_bytes = next_bytes;
        }
        log::info!(
            "tmux runtime {} buffering {} client events until presentation binds",
            self.id.as_u64(),
            events.len()
        );
        true
    }

    pub fn take_client_events(&self) -> Vec<TmuxClientEvent> {
        let mut inner = self.inner.lock();
        inner.pending_client_event_bytes = 0;
        std::mem::take(&mut inner.pending_client_events)
    }

    pub fn set_authoritative_shell_type(&self, shell_type: ShellType) -> Vec<String> {
        let mut inner = self.inner.lock();
        inner.shell_type = Some(shell_type);
        let pending = std::mem::take(&mut inner.pending_bootstrap_panes);
        let mut ready = Vec::new();
        for pane_id in pending {
            if inner.bootstrapped_panes.insert(pane_id.clone()) {
                ready.push(pane_id);
            }
        }
        ready
    }

    pub fn shell_type(&self) -> Option<ShellType> {
        self.inner.lock().shell_type
    }

    pub fn begin_pane_bootstrap(&self, pane_id: &str) -> Option<ShellType> {
        let mut inner = self.inner.lock();
        if inner.bootstrapped_panes.contains(pane_id) {
            return None;
        }
        if let Some(shell_type) = inner.shell_type {
            inner.bootstrapped_panes.insert(pane_id.to_owned());
            return Some(shell_type);
        }
        if !inner.pending_bootstrap_panes.iter().any(|id| id == pane_id) {
            inner.pending_bootstrap_panes.push_back(pane_id.to_owned());
        }
        None
    }

    pub fn pane_model(&self, pane_id: &str) -> Option<Arc<FairMutex<TerminalModel>>> {
        self.inner
            .lock()
            .panes
            .get(pane_id)
            .map(|sink| sink.model.clone())
    }

    pub fn start_app_bind_deadline(&self) {
        let mut inner = self.inner.lock();
        inner.presentation_ready = false;
        inner.app_bind_deadline = Some(instant::Instant::now() + APP_BIND_TIMEOUT);
    }

    pub fn mark_presentation_ready(&self) {
        let mut inner = self.inner.lock();
        inner.presentation_ready = true;
        inner.app_bind_deadline = None;
    }

    pub fn is_presentation_ready(&self) -> bool {
        self.inner.lock().presentation_ready
    }

    pub fn app_bind_deadline_elapsed(&self, now: instant::Instant) -> bool {
        let inner = self.inner.lock();
        if inner.presentation_ready {
            return false;
        }
        inner
            .app_bind_deadline
            .is_some_and(|deadline| now >= deadline)
    }

    #[cfg(test)]
    fn buffered_output(&self, pane_id: &str) -> Option<Vec<u8>> {
        self.inner.lock().buffers.get(pane_id).cloned()
    }
}

fn client_event_retained_bytes(event: &TmuxClientEvent) -> usize {
    let payload = match event {
        TmuxClientEvent::LayoutChange {
            window_id,
            layout,
            visible_layout,
            flags,
        } => {
            window_id.len()
                + layout.len()
                + visible_layout.as_ref().map(String::len).unwrap_or(0)
                + flags.as_ref().map(String::len).unwrap_or(0)
        }
        TmuxClientEvent::WindowAdd { window_id }
        | TmuxClientEvent::WindowClose { window_id }
        | TmuxClientEvent::SessionWindowChanged { window_id } => window_id.len(),
        TmuxClientEvent::WindowRenamed { window_id, name } => window_id.len() + name.len(),
        TmuxClientEvent::CommandEnd {
            payload,
            capture_pane,
            ..
        } => {
            payload.iter().map(String::len).sum::<usize>()
                + capture_pane.as_ref().map(String::len).unwrap_or(0)
        }
        TmuxClientEvent::PresentationUnready => 0,
    };
    PENDING_CLIENT_EVENT_OVERHEAD.saturating_add(payload)
}

fn clear_unregistered_buffers(inner: &mut Inner) {
    log::warn!("tmux unregistered pane output overflow; rolling back");
    inner.buffers.clear();
    inner.unregistered_bytes = 0;
}

fn feed_sink(sink: &mut PaneSink, bytes: &[u8]) {
    let mut writer = sink_writer();
    let mut model = sink.model.lock();
    sink.processor.parse_bytes(&mut *model, bytes, &mut writer);
    let _ = writer.flush();
    model.wake_for_tmux_output();
}

#[cfg(test)]
#[path = "bridge_tests.rs"]
mod tests;
