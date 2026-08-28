use std::collections::{HashMap, VecDeque};
use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

use parking_lot::{FairMutex, Mutex};
use warpui::WindowId;

use super::parser::PaneId;
use crate::terminal::TerminalModel;
use crate::terminal::model::ansi;
use crate::terminal::tmux::pane_bytes::sink_writer;

const MAX_BUFFERED_PANE_BYTES: usize = 64 * 1024;

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

    pub fn deliver_output(&self, pane_id: &PaneId, bytes: &[u8]) {
        let mut inner = self.inner.lock();
        if let Some(sink) = inner.panes.get_mut(pane_id.as_str()) {
            feed_sink(sink, bytes);
            return;
        }
        let buffer = inner
            .buffers
            .entry(pane_id.as_str().to_owned())
            .or_default();
        let overflow = (buffer.len() + bytes.len()).saturating_sub(MAX_BUFFERED_PANE_BYTES);
        if overflow > 0 {
            buffer.drain(..overflow);
        }
        buffer.extend_from_slice(bytes);
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

    #[cfg(test)]
    fn buffered_output(&self, pane_id: &str) -> Option<Vec<u8>> {
        self.inner.lock().buffers.get(pane_id).cloned()
    }
}

fn feed_sink(sink: &mut PaneSink, bytes: &[u8]) {
    let mut writer = sink_writer();
    let mut model = sink.model.lock();
    sink.processor.parse_bytes(&mut *model, bytes, &mut writer);
    let _ = writer.flush();
    model.wake_for_tmux_output();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runtime() -> TmuxRuntime {
        TmuxRuntime {
            id: TmuxInstanceId::new(),
            inner: Mutex::new(Inner {
                gateway_window: None,
                presentation_window: None,
                panes: HashMap::new(),
                buffers: HashMap::new(),
                pending_captures: VecDeque::new(),
            }),
            applying: AtomicBool::new(false),
        }
    }

    #[test]
    fn applying_flag_is_scoped() {
        let runtime = runtime();
        assert!(!runtime.is_applying());
        let value = runtime.with_applying(|| {
            assert!(runtime.is_applying());
            7
        });
        assert_eq!(value, 7);
        assert!(!runtime.is_applying());
    }

    #[test]
    fn capture_requests_are_fifo() {
        let runtime = runtime();
        runtime.note_capture("%4");
        runtime.note_capture("%7");
        assert_eq!(runtime.take_capture().as_deref(), Some("%4"));
        assert_eq!(runtime.take_capture().as_deref(), Some("%7"));
        assert_eq!(runtime.take_capture(), None);
    }

    #[test]
    fn two_instances_do_not_cross_pane_output() {
        let a = runtime();
        let b = runtime();
        a.deliver_output(&PaneId::from("%0"), b"from-a");
        b.deliver_output(&PaneId::from("%0"), b"from-b");
        assert_eq!(
            a.buffered_output("%0").as_deref(),
            Some(b"from-a".as_slice())
        );
        assert_eq!(
            b.buffered_output("%0").as_deref(),
            Some(b"from-b".as_slice())
        );
    }

    #[test]
    fn destructive_clear_is_instance_scoped() {
        let a = runtime();
        let b = runtime();
        a.note_capture("%0");
        b.note_capture("%0");
        a.deliver_output(&PaneId::from("%0"), b"keep-out-of-b");
        a.unregister();
        assert_eq!(a.take_capture(), None);
        assert_eq!(a.buffered_output("%0"), None);
        assert_eq!(b.take_capture().as_deref(), Some("%0"));
    }

    #[test]
    fn applying_on_one_instance_does_not_block_the_other() {
        let a = runtime();
        let b = runtime();
        a.with_applying(|| {
            assert!(a.is_applying());
            assert!(!b.is_applying());
        });
        assert!(!a.is_applying());
        assert!(!b.is_applying());
    }
}
