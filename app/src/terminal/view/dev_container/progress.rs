//! Piped BuildKit/devcontainer progress emits each byte-count snapshot as a
//! new LF line. CR overwrites the previous snapshot for the same vertex.

const ERASE_TO_EOL: &[u8] = b"\x1b[K";

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
    buf: Vec<u8>,
    last_identity: Option<Vec<u8>>,
    progress_open: bool,
}

impl ProgressCollapser {
    pub(crate) fn new() -> Self {
        Self {
            buf: Vec::new(),
            last_identity: None,
            progress_open: false,
        }
    }

    pub(crate) fn push(&mut self, chunk: &[u8]) -> Vec<u8> {
        self.buf.extend_from_slice(chunk);
        let mut out = Vec::new();
        while let Some(line) = take_lf_line(&mut self.buf) {
            self.emit_line(&line, &mut out);
        }
        out
    }

    pub(crate) fn finish(mut self) -> Vec<u8> {
        let mut out = Vec::new();
        if !self.buf.is_empty() {
            let line = std::mem::take(&mut self.buf);
            self.emit_line(&line, &mut out);
        }
        if self.progress_open {
            out.push(b'\n');
        }
        out
    }

    fn emit_line(&mut self, line: &[u8], out: &mut Vec<u8>) {
        if let Some(identity) = progress_identity(line) {
            if self.progress_open && self.last_identity.as_deref() == Some(identity.as_slice()) {
                out.push(b'\r');
                out.extend_from_slice(ERASE_TO_EOL);
                out.extend_from_slice(line);
            } else {
                if self.progress_open {
                    out.push(b'\n');
                }
                out.extend_from_slice(line);
                self.progress_open = true;
            }
            self.last_identity = Some(identity);
            return;
        }

        if self.progress_open {
            out.push(b'\n');
            self.progress_open = false;
            self.last_identity = None;
        }
        out.extend_from_slice(line);
        out.push(b'\n');
    }
}

fn take_lf_line(buf: &mut Vec<u8>) -> Option<Vec<u8>> {
    let i = buf.iter().position(|&b| b == b'\n')?;
    let mut line: Vec<u8> = buf.drain(..=i).collect();
    line.pop();
    if line.last() == Some(&b'\r') {
        line.pop();
    }
    Some(line)
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

#[cfg(test)]
#[path = "progress_tests.rs"]
mod tests;
