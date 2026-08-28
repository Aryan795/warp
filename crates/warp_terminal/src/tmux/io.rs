use std::borrow::Cow;
use std::collections::HashSet;
use std::time::{Duration, Instant};

use super::encode::{refresh_client_command, send_keys_command};
use super::parser::{ControlEvent, ControlModeParser, DecodeItem, PaneId, WindowId};

const START_PENDING_TIMEOUT: Duration = Duration::from_secs(8);
const DETACH_CLIENT: &[u8] = b"detach-client\n";

pub fn is_tmux_client_command(bytes: &[u8]) -> bool {
    bytes.starts_with(b"split-window")
        || bytes.starts_with(b"select-pane")
        || bytes.starts_with(b"kill-pane")
        || bytes.starts_with(b"resize-pane")
        || bytes.starts_with(b"refresh-client")
        || bytes.starts_with(b"new-window")
        || bytes.starts_with(b"select-window")
        || bytes.starts_with(b"kill-window")
        || bytes.starts_with(b"detach-client")
        || bytes.starts_with(b"pipe-pane")
        || bytes.starts_with(b"capture-pane")
}

pub fn is_tmux_cc_start(bytes: &[u8]) -> bool {
    let trimmed = bytes.trim_ascii_start();
    trimmed.starts_with(b"tmux -CC")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TmuxPhaseKind {
    Inactive,
    StartPending,
    InControl,
    OverflowRecovering,
}

enum TmuxPhase {
    Inactive,
    StartPending {
        pending_writes: Vec<Cow<'static, [u8]>>,
        pending_resize: Option<(usize, usize)>,
        started_at: Instant,
    },
    InControl {
        focused: Option<PaneId>,
        known_panes: HashSet<PaneId>,
        pending_writes: Vec<Cow<'static, [u8]>>,
        pending_resize: Option<(usize, usize)>,
    },
    OverflowRecovering {
        pending_writes: Vec<Cow<'static, [u8]>>,
    },
}

pub struct TmuxIoState {
    parser: ControlModeParser,
    phase: TmuxPhase,
}

impl Default for TmuxIoState {
    fn default() -> Self {
        Self::new()
    }
}

impl TmuxIoState {
    pub fn new() -> Self {
        Self {
            parser: ControlModeParser::new(),
            phase: TmuxPhase::Inactive,
        }
    }

    pub fn phase(&self) -> TmuxPhaseKind {
        match self.phase {
            TmuxPhase::Inactive => TmuxPhaseKind::Inactive,
            TmuxPhase::StartPending { .. } => TmuxPhaseKind::StartPending,
            TmuxPhase::InControl { .. } => TmuxPhaseKind::InControl,
            TmuxPhase::OverflowRecovering { .. } => TmuxPhaseKind::OverflowRecovering,
        }
    }

    pub fn focused_pane(&self) -> Option<&PaneId> {
        match &self.phase {
            TmuxPhase::InControl { focused, .. } => focused.as_ref(),
            _ => None,
        }
    }

    pub fn in_control(&self) -> bool {
        matches!(self.phase, TmuxPhase::InControl { .. })
    }

    pub fn enqueue_input(&mut self, input: Cow<'static, [u8]>) -> Vec<Cow<'static, [u8]>> {
        match &mut self.phase {
            TmuxPhase::Inactive => {
                let is_start = is_tmux_cc_start(&input);
                if is_start {
                    self.phase = TmuxPhase::StartPending {
                        pending_writes: Vec::new(),
                        pending_resize: None,
                        started_at: Instant::now(),
                    };
                }
                vec![input]
            }
            TmuxPhase::StartPending { pending_writes, .. }
            | TmuxPhase::OverflowRecovering { pending_writes } => {
                pending_writes.push(input);
                Vec::new()
            }
            TmuxPhase::InControl {
                focused,
                pending_writes,
                ..
            } => {
                if is_tmux_client_command(&input) {
                    return vec![input];
                }
                if let Some(pane) = focused {
                    let encoded = send_keys_command(pane, &input);
                    if encoded.is_empty() {
                        Vec::new()
                    } else {
                        vec![Cow::Owned(encoded)]
                    }
                } else {
                    pending_writes.push(input);
                    Vec::new()
                }
            }
        }
    }

    pub fn enqueue_resize(&mut self, columns: usize, rows: usize) -> Option<Cow<'static, [u8]>> {
        let in_control = matches!(self.phase, TmuxPhase::InControl { .. });
        match &mut self.phase {
            TmuxPhase::Inactive | TmuxPhase::OverflowRecovering { .. } => None,
            TmuxPhase::StartPending { pending_resize, .. }
            | TmuxPhase::InControl { pending_resize, .. } => {
                *pending_resize = Some((columns, rows));
                in_control.then(|| Cow::Owned(refresh_client_command(columns, rows).into_bytes()))
            }
        }
    }

    pub fn start_pending_remaining(&self) -> Option<Duration> {
        match &self.phase {
            TmuxPhase::StartPending { started_at, .. } => {
                Some(START_PENDING_TIMEOUT.saturating_sub(started_at.elapsed()))
            }
            _ => None,
        }
    }

    pub fn check_start_timeout(&mut self, now: Instant) -> Vec<TmuxFeedItem> {
        let TmuxPhase::StartPending { started_at, .. } = &self.phase else {
            return Vec::new();
        };
        if now.saturating_duration_since(*started_at) < START_PENDING_TIMEOUT {
            return Vec::new();
        }
        self.fail_start_pending()
    }

    pub fn feed(&mut self, bytes: &[u8]) -> Vec<TmuxFeedItem> {
        let mut items = Vec::new();
        let mut start_failed = false;
        for decoded in self.parser.decode(bytes) {
            if start_failed {
                continue;
            }
            match decoded {
                DecodeItem::Shell(shell) => {
                    let failed = matches!(self.phase, TmuxPhase::StartPending { .. })
                        && looks_like_start_failure(&shell);
                    items.push(TmuxFeedItem::Shell(shell));
                    if failed {
                        items.extend(self.fail_start_pending());
                        start_failed = true;
                    }
                }
                DecodeItem::Control(event) => items.extend(self.apply_control(event)),
            }
        }
        items
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TmuxFeedItem {
    Shell(Vec<u8>),
    EnteredControl {
        refresh_client: Option<String>,
    },
    PaneOutput {
        pane_id: PaneId,
        bytes: Vec<u8>,
    },
    LayoutChange {
        window_id: WindowId,
        layout: String,
        visible_layout: Option<String>,
        flags: Option<String>,
    },
    Focused(PaneId),
    EncodedPending(Vec<u8>),
    WindowAdd {
        window_id: WindowId,
    },
    WindowClose {
        window_id: WindowId,
    },
    WindowRenamed {
        window_id: WindowId,
        name: String,
    },
    SessionWindowChanged {
        window_id: WindowId,
    },
    CommandEnd {
        number: u64,
        error: bool,
        payload: Vec<String>,
    },
    Exited {
        replay: Vec<Cow<'static, [u8]>>,
    },
    OverflowRecovering {
        detach: Cow<'static, [u8]>,
    },
}

impl TmuxIoState {
    fn apply_control(&mut self, event: ControlEvent) -> Vec<TmuxFeedItem> {
        match event {
            ControlEvent::EnteredControlMode => {
                if matches!(self.phase, TmuxPhase::OverflowRecovering { .. }) {
                    return Vec::new();
                }
                let (pending_writes, pending_resize) = match &mut self.phase {
                    TmuxPhase::StartPending {
                        pending_writes,
                        pending_resize,
                        ..
                    } => (std::mem::take(pending_writes), pending_resize.take()),
                    TmuxPhase::Inactive => (Vec::new(), None),
                    TmuxPhase::InControl { .. } | TmuxPhase::OverflowRecovering { .. } => {
                        (Vec::new(), None)
                    }
                };
                self.phase = TmuxPhase::InControl {
                    focused: None,
                    known_panes: HashSet::new(),
                    pending_writes,
                    pending_resize,
                };
                vec![TmuxFeedItem::EnteredControl {
                    refresh_client: pending_resize
                        .map(|(columns, rows)| refresh_client_command(columns, rows)),
                }]
            }
            ControlEvent::PaneOutput { pane_id, bytes } => {
                self.note_pane(pane_id.clone());
                let mut items = vec![TmuxFeedItem::PaneOutput { pane_id, bytes }];
                items.extend(self.flush_pending_if_focused());
                items
            }
            ControlEvent::WindowPaneChanged { pane_id, .. } => {
                self.note_pane(pane_id.clone());
                self.set_focused(pane_id.clone());
                let mut items = vec![TmuxFeedItem::Focused(pane_id)];
                items.extend(self.flush_pending_if_focused());
                items
            }
            ControlEvent::LayoutChange {
                window_id,
                layout,
                visible_layout,
                flags,
            } => {
                if let Some(parsed) = super::layout::parse_window_layout(&layout) {
                    let ids = parsed.pane_ids();
                    for id in &ids {
                        self.note_pane(id.clone());
                    }
                    if ids.len() == 1
                        && let Some(id) = ids.into_iter().next()
                    {
                        self.set_focused(id);
                    }
                }
                let mut items = vec![TmuxFeedItem::LayoutChange {
                    window_id,
                    layout,
                    visible_layout,
                    flags,
                }];
                items.extend(self.flush_pending_if_focused());
                items
            }
            ControlEvent::WindowAdd { window_id } => {
                vec![TmuxFeedItem::WindowAdd { window_id }]
            }
            ControlEvent::WindowClose { window_id } => {
                vec![TmuxFeedItem::WindowClose { window_id }]
            }
            ControlEvent::WindowRenamed { window_id, name } => {
                vec![TmuxFeedItem::WindowRenamed { window_id, name }]
            }
            ControlEvent::SessionWindowChanged { window_id } => {
                vec![TmuxFeedItem::SessionWindowChanged { window_id }]
            }
            ControlEvent::CommandEnd {
                number,
                error,
                payload,
                ..
            } => {
                if payload.len() == 1
                    && let Some(pane_id) = payload.first().and_then(|line| parse_pane_id_line(line))
                {
                    self.note_pane(pane_id.clone());
                }
                vec![TmuxFeedItem::CommandEnd {
                    number,
                    error,
                    payload,
                }]
            }
            ControlEvent::CommandBegin { .. } => Vec::new(),
            ControlEvent::ProtocolOverflow => {
                let pending_writes = self.take_pending_writes();
                self.phase = TmuxPhase::OverflowRecovering { pending_writes };
                vec![TmuxFeedItem::OverflowRecovering {
                    detach: Cow::Borrowed(DETACH_CLIENT),
                }]
            }
            ControlEvent::Exit { .. } => {
                let replay = self.take_pending_writes();
                self.phase = TmuxPhase::Inactive;
                vec![TmuxFeedItem::Exited { replay }]
            }
        }
    }

    fn note_pane(&mut self, pane_id: PaneId) {
        if let TmuxPhase::InControl { known_panes, .. } = &mut self.phase {
            known_panes.insert(pane_id);
        }
    }

    fn set_focused(&mut self, pane_id: PaneId) {
        if let TmuxPhase::InControl {
            focused,
            known_panes,
            ..
        } = &mut self.phase
        {
            known_panes.insert(pane_id.clone());
            *focused = Some(pane_id);
        }
    }

    fn flush_pending_if_focused(&mut self) -> Vec<TmuxFeedItem> {
        let TmuxPhase::InControl {
            focused,
            pending_writes,
            ..
        } = &mut self.phase
        else {
            return Vec::new();
        };
        let Some(pane) = focused.clone() else {
            return Vec::new();
        };
        let pending = std::mem::take(pending_writes);
        pending
            .into_iter()
            .filter_map(|input| {
                let encoded = send_keys_command(&pane, &input);
                (!encoded.is_empty()).then_some(TmuxFeedItem::EncodedPending(encoded))
            })
            .collect()
    }

    fn take_pending_writes(&mut self) -> Vec<Cow<'static, [u8]>> {
        match &mut self.phase {
            TmuxPhase::StartPending { pending_writes, .. }
            | TmuxPhase::InControl { pending_writes, .. }
            | TmuxPhase::OverflowRecovering { pending_writes } => std::mem::take(pending_writes),
            TmuxPhase::Inactive => Vec::new(),
        }
    }

    fn fail_start_pending(&mut self) -> Vec<TmuxFeedItem> {
        let replay = self.take_pending_writes();
        self.phase = TmuxPhase::Inactive;
        self.parser = ControlModeParser::new();
        vec![TmuxFeedItem::Exited { replay }]
    }
}

fn looks_like_start_failure(shell: &[u8]) -> bool {
    let text = String::from_utf8_lossy(shell);
    let lower = text.to_ascii_lowercase();
    lower.contains("command not found")
        || lower.contains("tmux: unknown option")
        || lower.contains("tmux: invalid option")
        || lower.contains("not installed")
        || lower.contains("error connecting to")
        || lower.contains("no server running")
        || lower.contains("no such file or directory")
}

fn parse_pane_id_line(line: &str) -> Option<PaneId> {
    let line = line.trim();
    let digits = line.strip_prefix('%')?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(PaneId::from(line))
}

#[cfg(test)]
#[path = "io_tests.rs"]
mod tests;
