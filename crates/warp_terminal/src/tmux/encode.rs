use super::parser::PaneId;

const SEND_KEYS_CHUNK_BYTES: usize = 128;

/// Correlated query for the initial window/layout dump. tmux 3.6a `new-session -A`
/// enters control mode without `%layout-change` or `%window-pane-changed`.
pub const LIST_WINDOWS_LAYOUT_COMMAND: &str = "list-windows -F '#{window_id} #{window_layout}'\n";

pub fn refresh_client_command(columns: usize, rows: usize) -> String {
    format!("refresh-client -C {columns}x{rows}\n")
}

pub fn send_keys_command(pane_id: &PaneId, bytes: &[u8]) -> Vec<u8> {
    if bytes.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for chunk in bytes.chunks(SEND_KEYS_CHUNK_BYTES) {
        out.extend_from_slice(format!("send-keys -t {} -H", pane_id.as_str()).as_bytes());
        for byte in chunk {
            out.extend_from_slice(format!(" {byte:02x}").as_bytes());
        }
        out.push(b'\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn send_keys_encodes_hex_for_the_target_pane() {
        let encoded = send_keys_command(&PaneId::from("%3"), b"A\n");
        assert_eq!(encoded, b"send-keys -t %3 -H 41 0a\n");
    }
}
