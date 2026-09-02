//! Piped BuildKit/devcontainer progress emits each byte-count snapshot as a
//! new LF line. CR overwrites the previous snapshot for the same vertex.

const ERASE_TO_EOL: &[u8] = b"\x1b[K";
const DISABLE_LINE_WRAP: &[u8] = b"\x1b[?7l";
const ENABLE_LINE_WRAP: &[u8] = b"\x1b[?7h";
const PENDING_LINE_CAP: usize = 512;

const PROGRESS_VERBS: &[&[u8]] = &[
    b"extracting",
    b"downloading",
    b"transferring",
    b"exporting",
    b"writing",
    b"copying",
    b"unpacking",
    b"resolving",
    b"loading",
    b"importing",
];

pub(crate) struct ProgressCollapser {
    pending: Vec<u8>,
    last_identity: Option<Vec<u8>>,
    progress_open: bool,
    wrap_disabled: bool,
    pass_through_until_lf: bool,
    held_cr: bool,
}

impl ProgressCollapser {
    pub(crate) fn new() -> Self {
        Self {
            pending: Vec::new(),
            last_identity: None,
            progress_open: false,
            wrap_disabled: false,
            pass_through_until_lf: false,
            held_cr: false,
        }
    }

    pub(crate) fn push(&mut self, chunk: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        for &b in chunk {
            self.push_byte(b, &mut out);
        }
        out
    }

    pub(crate) fn finish(mut self) -> Vec<u8> {
        let mut out = Vec::new();
        if self.held_cr || !self.pending.is_empty() {
            let line = std::mem::take(&mut self.pending);
            self.emit_line(&line, &mut out);
        }
        self.end_progress_row(&mut out);
        out
    }

    fn push_byte(&mut self, b: u8, out: &mut Vec<u8>) {
        if self.pass_through_until_lf {
            out.push(b);
            if b == b'\n' {
                self.pass_through_until_lf = false;
            }
            return;
        }

        if b == b'\n' {
            self.held_cr = false;
            let line = std::mem::take(&mut self.pending);
            self.emit_line(&line, out);
            return;
        }

        if b == b'\r' {
            if self.held_cr {
                self.flush_held_cr_line(out);
            }
            self.held_cr = true;
            return;
        }

        if self.held_cr {
            self.flush_held_cr_line(out);
        }

        self.pending.push(b);
        if self.pending.len() >= PENDING_LINE_CAP || !pending_may_be_progress(&self.pending) {
            self.begin_pass_through(out);
        }
    }

    fn flush_held_cr_line(&mut self, out: &mut Vec<u8>) {
        self.held_cr = false;
        let line = std::mem::take(&mut self.pending);
        self.emit_line(&line, out);
    }

    fn begin_pass_through(&mut self, out: &mut Vec<u8>) {
        self.end_progress_row(out);
        out.append(&mut self.pending);
        self.pass_through_until_lf = true;
    }

    fn begin_progress_row(&mut self, out: &mut Vec<u8>) {
        if !self.wrap_disabled {
            out.extend_from_slice(DISABLE_LINE_WRAP);
            self.wrap_disabled = true;
        }
    }

    fn end_progress_row(&mut self, out: &mut Vec<u8>) {
        if !self.progress_open && !self.wrap_disabled {
            return;
        }
        if self.wrap_disabled {
            out.extend_from_slice(ENABLE_LINE_WRAP);
            self.wrap_disabled = false;
        }
        if self.progress_open {
            out.push(b'\n');
            self.progress_open = false;
        }
        self.last_identity = None;
    }

    fn emit_line(&mut self, line: &[u8], out: &mut Vec<u8>) {
        if let Some(identity) = progress_identity(line) {
            let display = progress_display_line(line);
            if self.progress_open && self.last_identity.as_deref() == Some(identity.as_slice()) {
                out.push(b'\r');
                out.extend_from_slice(ERASE_TO_EOL);
                out.extend_from_slice(display);
            } else {
                self.end_progress_row(out);
                self.begin_progress_row(out);
                out.extend_from_slice(display);
                self.progress_open = true;
            }
            self.last_identity = Some(identity);
            return;
        }

        self.end_progress_row(out);
        out.extend_from_slice(line);
        out.push(b'\n');
    }
}

fn pending_may_be_progress(pending: &[u8]) -> bool {
    let rest = match timestamp_rest(pending) {
        Some(rest) => rest,
        None => return true,
    };
    may_be_buildkit_progress(rest) || may_be_classic_layer_progress(rest)
}

fn timestamp_rest(line: &[u8]) -> Option<&[u8]> {
    if line.first() != Some(&b'[') {
        return Some(line);
    }
    let end = line.iter().position(|&b| b == b']')?;
    let rest = &line[end + 1..];
    Some(rest.strip_prefix(b" ").unwrap_or(rest))
}

fn progress_display_line(line: &[u8]) -> &[u8] {
    match timestamp_rest(line) {
        Some(rest) if !rest.is_empty() => rest,
        _ => line,
    }
}

fn may_be_buildkit_progress(line: &[u8]) -> bool {
    if line.is_empty() {
        return true;
    }
    if line[0] != b'#' {
        return false;
    }
    let rest = &line[1..];
    if rest.is_empty() {
        return true;
    }
    let digits_end = rest
        .iter()
        .position(|&b| !b.is_ascii_digit())
        .unwrap_or(rest.len());
    if digits_end == 0 {
        return false;
    }
    if digits_end == rest.len() {
        return true;
    }
    if rest[digits_end] != b' ' {
        return false;
    }
    let after_id = &rest[digits_end + 1..];
    if after_id.is_empty() {
        return true;
    }
    match after_id.iter().position(|&b| b == b' ') {
        None => is_progress_verb_prefix(after_id),
        Some(verb_end) => is_progress_verb(&after_id[..verb_end]),
    }
}

fn may_be_classic_layer_progress(line: &[u8]) -> bool {
    if line.is_empty() {
        return true;
    }
    match line.iter().position(|&b| b == b':') {
        None => line.iter().all(|b| b.is_ascii_hexdigit()),
        Some(colon) => {
            let id = &line[..colon];
            if id.len() < 12 || !id.iter().all(|b| b.is_ascii_hexdigit()) {
                return false;
            }
            let mut rest = &line[colon + 1..];
            if rest.first() == Some(&b' ') {
                rest = &rest[1..];
            }
            if rest.is_empty() {
                return true;
            }
            match rest.iter().position(|&b| b == b' ') {
                None => is_progress_verb_prefix(rest),
                Some(verb_end) => is_progress_verb(&rest[..verb_end]),
            }
        }
    }
}

fn progress_identity(line: &[u8]) -> Option<Vec<u8>> {
    let line = strip_cli_timestamp(line);
    buildkit_progress_identity(line).or_else(|| classic_layer_progress_identity(line))
}

fn strip_cli_timestamp(line: &[u8]) -> &[u8] {
    if line.first() != Some(&b'[') {
        return line;
    }
    let Some(end) = line.iter().position(|&b| b == b']') else {
        return line;
    };
    let rest = &line[end + 1..];
    rest.strip_prefix(b" ").unwrap_or(rest)
}

fn buildkit_progress_identity(line: &[u8]) -> Option<Vec<u8>> {
    if !line.starts_with(b"#") {
        return None;
    }
    let rest = &line[1..];
    let digits_end = rest.iter().position(|&b| !b.is_ascii_digit())?;
    if digits_end == 0 || rest.get(digits_end) != Some(&b' ') {
        return None;
    }
    let after_id = &rest[digits_end + 1..];
    let verb_end = after_id.iter().position(|&b| b == b' ')?;
    let verb = &after_id[..verb_end];
    if !is_progress_verb(verb) {
        return None;
    }
    let after_verb = &after_id[verb_end + 1..];
    let object_end = after_verb
        .iter()
        .position(|&b| b == b' ')
        .unwrap_or(after_verb.len());
    if object_end == 0 {
        return None;
    }
    Some(line[..1 + digits_end + 1 + verb_end + 1 + object_end].to_vec())
}

fn classic_layer_progress_identity(line: &[u8]) -> Option<Vec<u8>> {
    let colon = line.iter().position(|&b| b == b':')?;
    let id = &line[..colon];
    if id.len() < 12 || !id.iter().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let mut rest = line.get(colon + 1..)?;
    let mut prefix_len = colon + 1;
    if rest.first() == Some(&b' ') {
        rest = &rest[1..];
        prefix_len += 1;
    }
    let verb_end = rest.iter().position(|&b| b == b' ').unwrap_or(rest.len());
    let verb = &rest[..verb_end];
    if !is_progress_verb(verb) {
        return None;
    }
    Some(line[..prefix_len + verb_end].to_vec())
}

fn is_progress_verb(verb: &[u8]) -> bool {
    PROGRESS_VERBS
        .iter()
        .any(|candidate| verb.eq_ignore_ascii_case(candidate))
}

fn is_progress_verb_prefix(verb: &[u8]) -> bool {
    PROGRESS_VERBS.iter().any(|candidate| {
        verb.len() <= candidate.len() && candidate[..verb.len()].eq_ignore_ascii_case(verb)
    })
}

#[cfg(test)]
#[path = "progress_tests.rs"]
mod tests;
