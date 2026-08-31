use super::*;
use crate::event::Event;

fn key_payload(key: &str, code: &str) -> DesktopKeyboardPayload {
    DesktopKeyboardPayload {
        kind: DomKeyEventKind::Down,
        key: key.to_string(),
        code: code.to_string(),
        ctrl: false,
        alt: false,
        shift: false,
        meta: false,
        is_composing: false,
    }
}

#[test]
fn unmodified_printable_key_produces_keydown_and_fallback_chars() {
    let payload = key_payload("a", "KeyA");
    let Some(KeyConversion::Down { event, chars }) = convert_key(&payload) else {
        panic!("expected a Down conversion");
    };
    let Event::KeyDown {
        keystroke,
        chars: key_down_chars,
        ..
    } = event
    else {
        panic!("expected a KeyDown event");
    };
    assert_eq!(keystroke.key, "a");
    assert!(!keystroke.has_any_modifier());
    assert_eq!(key_down_chars, "a");
    assert_eq!(chars.as_deref(), Some("a"));
}

#[test]
fn shifted_letter_uppercases_the_keystroke_key() {
    let mut payload = key_payload("A", "KeyA");
    payload.shift = true;
    let Some(KeyConversion::Down { event, .. }) = convert_key(&payload) else {
        panic!("expected a Down conversion");
    };
    let Event::KeyDown { keystroke, .. } = event else {
        panic!("expected a KeyDown event");
    };
    assert_eq!(keystroke.key, "A");
    assert!(keystroke.shift);
}

#[test]
fn ctrl_c_carries_the_control_byte_on_keydown_but_not_in_fallback_chars() {
    let mut payload = key_payload("c", "KeyC");
    payload.ctrl = true;
    let Some(KeyConversion::Down { event, chars }) = convert_key(&payload) else {
        panic!("expected a Down conversion");
    };
    let Event::KeyDown {
        keystroke,
        chars: key_down_chars,
        ..
    } = event
    else {
        panic!("expected a KeyDown event");
    };
    assert_eq!(keystroke.key, "c");
    assert!(keystroke.ctrl);
    // Raw terminal input reads this field directly to write the interrupt byte to the pty.
    assert_eq!(key_down_chars, "\u{3}");
    // The unhandled-keydown fallback should never insert the literal "c" instead.
    assert_eq!(chars.as_deref(), Some("c"));
}

#[test]
fn named_key_produces_keydown_with_no_fallback_chars() {
    let payload = key_payload("Enter", "Enter");
    let Some(KeyConversion::Down { event, chars }) = convert_key(&payload) else {
        panic!("expected a Down conversion");
    };
    let Event::KeyDown { keystroke, .. } = event else {
        panic!("expected a KeyDown event");
    };
    assert_eq!(keystroke.key, "enter");
    assert_eq!(chars, None);
}

#[test]
fn composing_key_is_left_to_the_browser() {
    let mut payload = key_payload("a", "KeyA");
    payload.is_composing = true;
    assert!(convert_key(&payload).is_none());
}

#[test]
fn browser_paste_shortcut_is_left_to_the_browser() {
    let mut payload = key_payload("v", "KeyV");
    if crate::platform::OperatingSystem::get().is_mac() {
        payload.meta = true;
    } else {
        payload.ctrl = true;
    }
    assert!(convert_key(&payload).is_none());
}

#[test]
fn modifier_keydown_and_keyup_report_press_and_release() {
    let down = DesktopKeyboardPayload {
        kind: DomKeyEventKind::Down,
        shift: true,
        ..key_payload("Shift", "ShiftLeft")
    };
    let Some(KeyConversion::ModifierChanged { key_code, state }) = convert_key(&down) else {
        panic!("expected a ModifierChanged conversion");
    };
    assert_eq!(key_code, crate::platform::keyboard::KeyCode::ShiftLeft);
    assert!(matches!(state, crate::event::KeyState::Pressed));

    let up = DesktopKeyboardPayload {
        kind: DomKeyEventKind::Up,
        shift: false,
        ..key_payload("Shift", "ShiftLeft")
    };
    let Some(KeyConversion::ModifierChanged { state, .. }) = convert_key(&up) else {
        panic!("expected a ModifierChanged conversion");
    };
    assert!(matches!(state, crate::event::KeyState::Released));
}

#[test]
fn unmodified_non_modifier_keyup_is_ignored() {
    let payload = DesktopKeyboardPayload {
        kind: DomKeyEventKind::Up,
        ..key_payload("a", "KeyA")
    };
    assert!(convert_key(&payload).is_none());
}

#[test]
fn classify_input_type_covers_insert_and_delete_directions() {
    assert_eq!(
        classify_input_type("insertText"),
        InputClassification::Insert
    );
    assert_eq!(
        classify_input_type("insertCompositionText"),
        InputClassification::Insert
    );
    assert_eq!(
        classify_input_type("deleteContentBackward"),
        InputClassification::Delete(DeleteDirection::Backward)
    );
    assert_eq!(
        classify_input_type("deleteContentForward"),
        InputClassification::Delete(DeleteDirection::Forward)
    );
    assert_eq!(
        classify_input_type("formatBold"),
        InputClassification::Unsupported
    );
}

#[test]
fn extract_inserted_text_diffs_against_the_sentinel() {
    assert_eq!(
        extract_inserted_text(SENTINEL, " hello"),
        Some("hello".to_string())
    );
    assert_eq!(extract_inserted_text(SENTINEL, " "), None);
    assert_eq!(extract_inserted_text(SENTINEL, "hello"), None);
}

#[test]
fn composition_selection_range_strips_the_sentinel_and_clamps() {
    // Sentinel is 1 UTF-16 code unit; marked text is "ab" (2 units).
    assert_eq!(composition_selection_range(1, 2, 1, 3), 0..2);
    // A stale selection that runs past the marked text clamps to its end.
    assert_eq!(composition_selection_range(1, 2, 1, 10), 0..2);
    // A selection that hasn't caught up with the sentinel clamps to zero.
    assert_eq!(composition_selection_range(1, 2, 0, 0), 0..0);
}
