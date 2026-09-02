//! `--log-format json` makes the Dev Containers CLI set `isTTY` even when stdout
//! is piped, so Docker keeps native CR/cursor redraws. Those bytes arrive as
//! `raw` log events; structured `text` events are separate JSON lines.

use super::newline::NewlineNormalizer;

pub(crate) struct JsonlDecoder {
    pending: Vec<u8>,
}

impl JsonlDecoder {
    pub(crate) fn new() -> Self {
        Self {
            pending: Vec::new(),
        }
    }

    pub(crate) fn push(&mut self, chunk: &[u8]) -> Vec<u8> {
        self.pending.extend_from_slice(chunk);
        let mut out = Vec::new();
        while let Some(newline_at) = self.pending.iter().position(|&b| b == b'\n') {
            let mut line: Vec<u8> = self.pending.drain(..=newline_at).collect();
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            out.extend(decode_line(&line));
        }
        out
    }

    pub(crate) fn finish(self) -> Vec<u8> {
        if self.pending.is_empty() {
            Vec::new()
        } else {
            decode_line(&self.pending)
        }
    }
}

fn decode_line(line: &[u8]) -> Vec<u8> {
    if line.is_empty() {
        return Vec::new();
    }
    match classify(line) {
        Classified::Raw(text) => text.into_bytes(),
        Classified::Text(text) => render_structured_text(&text),
        Classified::Ignore => Vec::new(),
        Classified::Leftover => render_structured_text(&String::from_utf8_lossy(line)),
    }
}

enum Classified {
    Raw(String),
    Text(String),
    Ignore,
    Leftover,
}

fn classify(line: &[u8]) -> Classified {
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(line) else {
        return Classified::Leftover;
    };
    let Some(obj) = value.as_object() else {
        return Classified::Leftover;
    };
    let Some(event_type) = obj.get("type").and_then(|v| v.as_str()) else {
        return Classified::Leftover;
    };
    match event_type {
        "raw" => match text_field(obj) {
            Some(text) if !text.is_empty() => Classified::Raw(text),
            _ => Classified::Ignore,
        },
        "text" | "start" => match text_field(obj) {
            Some(text) if !text.is_empty() => Classified::Text(text),
            _ => Classified::Ignore,
        },
        "stop" | "progress" => Classified::Ignore,
        _ => match text_field(obj) {
            Some(text) if !text.is_empty() => Classified::Text(text),
            _ => Classified::Ignore,
        },
    }
}

fn text_field(obj: &serde_json::Map<String, serde_json::Value>) -> Option<String> {
    obj.get("text")?.as_str().map(str::to_owned)
}

fn render_structured_text(text: &str) -> Vec<u8> {
    let mut normalizer = NewlineNormalizer::new();
    let mut out = normalizer.push(text.as_bytes());
    if !text.ends_with('\n') {
        out.extend(normalizer.push(b"\n"));
    }
    out.extend(normalizer.finish());
    out
}

#[cfg(test)]
#[path = "jsonl_tests.rs"]
mod tests;
