//! Line-oriented tmux control-mode parser for `-CC` notifications.
//!
//! Raw `pipe-pane -O` journal bytes are not control-mode lines and must not be fed through this
//! parser.

/// DCS sequence tmux emits when a client enters control mode (`tmux -CC`).
pub const CONTROL_MODE_DCS: &[u8] = b"\x1bP1000p";

/// Identity of a tmux pane as reported by control mode (`%0`, `%1`, ...).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PaneId(String);

impl PaneId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for PaneId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

/// Identity of a tmux window as reported by control mode (`@0`, `@1`, ...).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct WindowId(String);

impl WindowId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for WindowId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

/// Parsed control-mode notifications. Protocol chatter that is not pane output is either
/// represented here or dropped; it is never treated as VT bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlEvent {
    EnteredControlMode,
    CommandBegin {
        time: u64,
        number: u64,
    },
    CommandEnd {
        time: u64,
        number: u64,
        error: bool,
        payload: Vec<String>,
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
    WindowPaneChanged {
        window_id: WindowId,
        pane_id: PaneId,
    },
    Exit {
        reason: Option<String>,
    },
}

enum State {
    SeekingDcs {
        pending: Vec<u8>,
    },
    InControlMode {
        line: Vec<u8>,
        in_command_reply: bool,
        reply_payload: Vec<String>,
    },
}

/// Incremental parser for tmux control-mode byte streams.
pub struct ControlModeParser {
    state: State,
}

impl Default for ControlModeParser {
    fn default() -> Self {
        Self::new()
    }
}

impl ControlModeParser {
    pub fn new() -> Self {
        Self {
            state: State::SeekingDcs {
                pending: Vec::new(),
            },
        }
    }

    pub fn is_in_control_mode(&self) -> bool {
        matches!(self.state, State::InControlMode { .. })
    }

    /// Consume a chunk of bytes and return zero or more complete control events.
    pub fn push(&mut self, bytes: &[u8]) -> Vec<ControlEvent> {
        let mut events = Vec::new();
        match &mut self.state {
            State::SeekingDcs { pending } => {
                pending.extend_from_slice(bytes);
                if let Some(index) = find_subslice(pending, CONTROL_MODE_DCS) {
                    let remainder = pending.split_off(index + CONTROL_MODE_DCS.len());
                    self.state = State::InControlMode {
                        line: remainder,
                        in_command_reply: false,
                        reply_payload: Vec::new(),
                    };
                    events.push(ControlEvent::EnteredControlMode);
                    events.extend(self.drain_control_lines());
                } else if pending.len() > CONTROL_MODE_DCS.len() {
                    let keep = CONTROL_MODE_DCS.len() - 1;
                    let drain_to = pending.len() - keep;
                    pending.drain(..drain_to);
                }
            }
            State::InControlMode { line, .. } => {
                line.extend_from_slice(bytes);
                events.extend(self.drain_control_lines());
            }
        }
        events
    }

    fn drain_control_lines(&mut self) -> Vec<ControlEvent> {
        let State::InControlMode {
            line,
            in_command_reply,
            reply_payload,
        } = &mut self.state
        else {
            return Vec::new();
        };

        let mut events = Vec::new();
        loop {
            let Some(newline_at) = line.iter().position(|&b| b == b'\n') else {
                break;
            };
            let mut raw_line = line.drain(..=newline_at).collect::<Vec<_>>();
            raw_line.pop();
            if raw_line.last() == Some(&b'\r') {
                raw_line.pop();
            }
            if let Some(event) = parse_control_line(&raw_line, in_command_reply, reply_payload) {
                events.push(event);
            }
        }
        events
    }
}

fn parse_control_line(
    line: &[u8],
    in_command_reply: &mut bool,
    reply_payload: &mut Vec<String>,
) -> Option<ControlEvent> {
    let text = std::str::from_utf8(line).ok()?;
    if text.is_empty() {
        return None;
    }

    if let Some(rest) = text.strip_prefix("%begin ") {
        let (time, number) = parse_begin_end_args(rest)?;
        *in_command_reply = true;
        reply_payload.clear();
        return Some(ControlEvent::CommandBegin { time, number });
    }
    if let Some(rest) = text.strip_prefix("%end ") {
        let (time, number) = parse_begin_end_args(rest)?;
        *in_command_reply = false;
        let payload = std::mem::take(reply_payload);
        return Some(ControlEvent::CommandEnd {
            time,
            number,
            error: false,
            payload,
        });
    }
    if let Some(rest) = text.strip_prefix("%error ") {
        let (time, number) = parse_begin_end_args(rest)?;
        *in_command_reply = false;
        let payload = std::mem::take(reply_payload);
        return Some(ControlEvent::CommandEnd {
            time,
            number,
            error: true,
            payload,
        });
    }
    if let Some(rest) = text.strip_prefix("%exit") {
        let reason = rest.trim();
        let reason = if reason.is_empty() {
            None
        } else {
            Some(reason.trim_start().to_owned())
        };
        return Some(ControlEvent::Exit { reason });
    }

    if *in_command_reply {
        reply_payload.push(text.to_owned());
        return None;
    }

    if let Some(rest) = text.strip_prefix("%output ") {
        return parse_output_line(rest);
    }
    if let Some(rest) = text.strip_prefix("%layout-change ") {
        return parse_layout_change(rest);
    }
    if let Some(rest) = text.strip_prefix("%window-add ") {
        return parse_window_id_event(rest).map(|window_id| ControlEvent::WindowAdd { window_id });
    }
    if let Some(rest) = text.strip_prefix("%window-close ") {
        return parse_window_id_event(rest)
            .map(|window_id| ControlEvent::WindowClose { window_id });
    }
    if let Some(rest) = text.strip_prefix("%window-renamed ") {
        return parse_window_renamed(rest);
    }
    if let Some(rest) = text.strip_prefix("%session-window-changed ") {
        return parse_session_window_changed(rest);
    }
    if let Some(rest) = text.strip_prefix("%window-pane-changed ") {
        return parse_window_pane_changed(rest);
    }

    None
}

fn parse_begin_end_args(rest: &str) -> Option<(u64, u64)> {
    let mut parts = rest.split_whitespace();
    let time = parts.next()?.parse().ok()?;
    let number = parts.next()?.parse().ok()?;
    Some((time, number))
}

fn parse_output_line(rest: &str) -> Option<ControlEvent> {
    let rest = rest.strip_prefix('%').unwrap_or(rest);
    let (id, data) = match rest.split_once(' ') {
        Some((id, data)) => (id, data.as_bytes()),
        None => (rest, b"".as_slice()),
    };
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(ControlEvent::PaneOutput {
        pane_id: PaneId(format!("%{id}")),
        bytes: octal_unescape(data),
    })
}

fn parse_window_id_event(rest: &str) -> Option<WindowId> {
    let id = rest.split_whitespace().next()?;
    parse_window_id(id)
}

fn parse_window_id(id: &str) -> Option<WindowId> {
    let digits = id.strip_prefix('@')?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(WindowId(format!("@{digits}")))
}

fn parse_pane_id_token(id: &str) -> Option<PaneId> {
    let digits = id.strip_prefix('%')?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(PaneId(format!("%{digits}")))
}

fn parse_layout_change(rest: &str) -> Option<ControlEvent> {
    let mut parts = rest.splitn(4, ' ');
    let window_id = parse_window_id(parts.next()?)?;
    let layout = parts.next()?.to_owned();
    let visible_layout = parts.next().map(str::to_owned);
    let flags = parts.next().map(str::to_owned);
    Some(ControlEvent::LayoutChange {
        window_id,
        layout,
        visible_layout,
        flags,
    })
}

fn parse_window_renamed(rest: &str) -> Option<ControlEvent> {
    let (id, name) = rest.split_once(' ')?;
    Some(ControlEvent::WindowRenamed {
        window_id: parse_window_id(id)?,
        name: name.to_owned(),
    })
}

fn parse_session_window_changed(rest: &str) -> Option<ControlEvent> {
    let mut parts = rest.split_whitespace();
    let _session = parts.next()?;
    let window_id = parse_window_id(parts.next()?)?;
    Some(ControlEvent::SessionWindowChanged { window_id })
}

fn parse_window_pane_changed(rest: &str) -> Option<ControlEvent> {
    let mut parts = rest.split_whitespace();
    let window_id = parse_window_id(parts.next()?)?;
    let pane_id = parse_pane_id_token(parts.next()?)?;
    Some(ControlEvent::WindowPaneChanged { window_id, pane_id })
}

/// Decode tmux control-mode octal escapes (`\xxx` for bytes `< 0x20`, `\\`, or `>= 0x7f`).
pub fn octal_unescape(input: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len());
    let mut i = 0;
    while i < input.len() {
        if input[i] == b'\\' && i + 3 < input.len() {
            let a = input[i + 1];
            let b = input[i + 2];
            let c = input[i + 3];
            if is_octal_digit(a) && is_octal_digit(b) && is_octal_digit(c) {
                out.push(((a - b'0') << 6) | ((b - b'0') << 3) | (c - b'0'));
                i += 4;
                continue;
            }
        }
        out.push(input[i]);
        i += 1;
    }
    out
}

fn is_octal_digit(b: u8) -> bool {
    (b'0'..=b'7').contains(&b)
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[cfg(test)]
#[path = "parser_tests.rs"]
mod tests;
