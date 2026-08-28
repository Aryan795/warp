use std::collections::{HashMap, VecDeque};
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use parking_lot::{FairMutex, Mutex};
use warpui::WindowId;

use super::parser::PaneId;
use crate::terminal::TerminalModel;
use crate::terminal::model::ansi;
use crate::terminal::tmux::pane_bytes::sink_writer;

const MAX_BUFFERED_PANE_BYTES: usize = 64 * 1024;

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

/// Shared between the gateway PTY thread and presentation UI.
pub struct TmuxRuntime {
    inner: Mutex<Inner>,
    applying: AtomicBool,
}

impl TmuxRuntime {
    pub fn global() -> &'static Self {
        static RUNTIME: OnceLock<TmuxRuntime> = OnceLock::new();
        RUNTIME.get_or_init(|| Self {
            inner: Mutex::new(Inner {
                gateway_window: None,
                presentation_window: None,
                panes: HashMap::new(),
                buffers: HashMap::new(),
                pending_captures: VecDeque::new(),
            }),
            applying: AtomicBool::new(false),
        })
    }

    pub fn set_gateway_window(&self, window_id: WindowId) {
        self.inner.lock().gateway_window = Some(window_id);
    }

    pub fn gateway_window(&self) -> Option<WindowId> {
        self.inner.lock().gateway_window
    }

    pub fn set_presentation_window(&self, window_id: WindowId) {
        self.inner.lock().presentation_window = Some(window_id);
    }

    pub fn presentation_window(&self) -> Option<WindowId> {
        self.inner.lock().presentation_window
    }

    pub fn clear_session(&self) {
        let mut inner = self.inner.lock();
        inner.gateway_window = None;
        inner.presentation_window = None;
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

    #[test]
    fn applying_flag_is_scoped() {
        let runtime = TmuxRuntime {
            inner: Mutex::new(Inner {
                gateway_window: None,
                presentation_window: None,
                panes: HashMap::new(),
                buffers: HashMap::new(),
                pending_captures: VecDeque::new(),
            }),
            applying: AtomicBool::new(false),
        };
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
        let runtime = TmuxRuntime {
            inner: Mutex::new(Inner {
                gateway_window: None,
                presentation_window: None,
                panes: HashMap::new(),
                buffers: HashMap::new(),
                pending_captures: VecDeque::new(),
            }),
            applying: AtomicBool::new(false),
        };
        runtime.note_capture("%4");
        runtime.note_capture("%7");
        assert_eq!(runtime.take_capture().as_deref(), Some("%4"));
        assert_eq!(runtime.take_capture().as_deref(), Some("%7"));
        assert_eq!(runtime.take_capture(), None);
    }
}
