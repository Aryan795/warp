use super::*;

// Regression coverage for GH#15196 / CSAT-10277: on macOS, while a non-Latin input source (e.g.
// Korean/Hangul) is active, `charactersIgnoringModifiers` can be empty or a non-ASCII IME
// composition character for a Ctrl-modified key, even though a Ctrl chord is never IME
// composition input. These helpers recover the physical key (and its C0 control byte) in that
// case; `crates/warpui/src/platform/mac/event.rs` wires them into the macOS `KeyDown` event
// conversion, which cannot be unit-tested directly outside of a macOS host since it requires a
// live `NSEvent`.

#[test]
fn ctrl_chord_physical_letter_maps_letter_keys() {
    assert_eq!(ctrl_chord_physical_letter(KeyCode::KeyJ), Some("j"));
    assert_eq!(ctrl_chord_physical_letter(KeyCode::KeyA), Some("a"));
    assert_eq!(ctrl_chord_physical_letter(KeyCode::KeyZ), Some("z"));
}

#[test]
fn ctrl_chord_physical_letter_ignores_non_letter_keys() {
    assert_eq!(ctrl_chord_physical_letter(KeyCode::Digit1), None);
    assert_eq!(ctrl_chord_physical_letter(KeyCode::Space), None);
    assert_eq!(ctrl_chord_physical_letter(KeyCode::Enter), None);
}

#[test]
fn ctrl_chord_fallback_not_needed_without_ctrl() {
    // Without Ctrl held, always defer to whatever the input source produced (including
    // nothing), since this is ordinary IME composition input.
    assert!(!ctrl_chord_needs_physical_key_fallback(false, None));
    assert!(!ctrl_chord_needs_physical_key_fallback(
        false,
        Some('\u{314F}')
    ));
}

#[test]
fn ctrl_chord_fallback_not_needed_for_ascii_result() {
    // English/ABC input source: Ctrl+J already produces an ASCII character, so the existing
    // (possibly layout-remapped, e.g. Dvorak/AZERTY) behavior should be preserved.
    assert!(!ctrl_chord_needs_physical_key_fallback(true, Some('j')));
}

#[test]
fn ctrl_chord_fallback_needed_for_empty_or_non_ascii_result() {
    // Korean/Hangul input source active: `charactersIgnoringModifiers` is empty, or a Hangul
    // jamo (U+314F 'ㅏ') rather than 'j'.
    assert!(ctrl_chord_needs_physical_key_fallback(true, None));
    assert!(ctrl_chord_needs_physical_key_fallback(
        true,
        Some('\u{314F}')
    ));
}

#[test]
fn ctrl_letter_to_control_char_maps_ctrl_j_to_line_feed() {
    assert_eq!(ctrl_letter_to_control_char("j"), Some('\u{0A}'));
}

#[test]
fn ctrl_letter_to_control_char_covers_full_alphabet() {
    assert_eq!(ctrl_letter_to_control_char("a"), Some('\u{01}'));
    assert_eq!(ctrl_letter_to_control_char("z"), Some('\u{1A}'));
}

#[test]
fn ctrl_letter_to_control_char_rejects_non_letters() {
    assert_eq!(ctrl_letter_to_control_char("1"), None);
    assert_eq!(ctrl_letter_to_control_char(""), None);
    assert_eq!(ctrl_letter_to_control_char("ab"), None);
}

/// End-to-end regression test for the decision logic behind GH#15196 / CSAT-10277: given a
/// Ctrl-modified `KeyJ` press where the active input source produced nothing usable (as happens
/// under a Hangul input source), the resolved key must still be `"j"` and its control byte must
/// still be the `0x0A` (LF) that Warp's own editor binds `ctrl-j` to and that raw-mode PTY
/// clients expect. This exercises the same three helpers that
/// `crates/warpui/src/platform/mac/event.rs` composes in its (untestable outside macOS)
/// `NSEvent` conversion.
#[test]
fn ctrl_j_resolves_to_newline_when_input_source_produces_nothing_usable() {
    let ctrl_held = true;
    // Simulates `charactersIgnoringModifiers` under a Hangul input source: empty.
    let ime_first_char: Option<char> = None;

    assert!(ctrl_chord_needs_physical_key_fallback(
        ctrl_held,
        ime_first_char
    ));

    let physical_letter = ctrl_chord_physical_letter(KeyCode::KeyJ).expect("KeyJ is a letter key");
    assert_eq!(physical_letter, "j");

    let control_char =
        ctrl_letter_to_control_char(physical_letter).expect("letter fallback is always a-z");
    assert_eq!(control_char, '\u{0A}');
}
