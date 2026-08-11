use computer_use::Target;
use warp_multi_agent_api as api;

use crate::agent::action::AIAgentActionType;
use crate::agent::convert::ToolToAIAgentActionError;

// The deprecated legacy field must still be set to construct this struct
// literal (it is a plain, non-optional proto3 field); tests exercise the
// current `playback_speed` field instead.
#[allow(deprecated)]
fn start_recording(
    target: Option<api::message::tool_call::ComputerUseTarget>,
) -> api::message::tool_call::StartRecording {
    api::message::tool_call::StartRecording {
        frame_rate: 15,
        limits: None,
        summary: String::new(),
        playback_speed_multiplier: 0,
        target,
        description: String::new(),
        playback_speed: 0.0,
    }
}

fn window_target(window_id: &str) -> api::message::tool_call::ComputerUseTarget {
    use api::message::tool_call::computer_use_target::{Target as ApiTarget, Window};
    api::message::tool_call::ComputerUseTarget {
        target: Some(ApiTarget::Window(Window {
            window_id: window_id.to_string(),
            pid: 7,
        })),
    }
}

#[test]
fn start_recording_parses_valid_window_target() {
    let action = AIAgentActionType::try_from(start_recording(Some(window_target("12345"))))
        .expect("valid window id should convert");
    match action {
        AIAgentActionType::StartRecording {
            window: Some(Target::Window { window_id, pid }),
            ..
        } => {
            assert_eq!(window_id, 12345);
            assert_eq!(pid, 7);
        }
        other => panic!("expected a window recording target, got {other:?}"),
    }
}

#[test]
fn start_recording_rejects_unparseable_window_id() {
    // A malformed window id must error before capture starts rather than silently
    // falling back to whole-screen recording.
    let err = AIAgentActionType::try_from(start_recording(Some(window_target("not-a-window"))))
        .expect_err("an unparseable window id should be rejected");
    assert!(
        matches!(&err, ToolToAIAgentActionError::InvalidRecordingWindowId(id) if id == "not-a-window"),
        "expected InvalidRecordingWindowId, got {err:?}"
    );
}

#[test]
fn start_recording_without_target_records_whole_screen() {
    let action =
        AIAgentActionType::try_from(start_recording(None)).expect("absent target should convert");
    assert!(matches!(
        action,
        AIAgentActionType::StartRecording { window: None, .. }
    ));
}

#[test]
fn start_recording_carries_fractional_playback_speed_above_one() {
    let mut tool_call = start_recording(None);
    tool_call.playback_speed = 1.5;
    let action =
        AIAgentActionType::try_from(tool_call).expect("valid start_recording should convert");
    match action {
        AIAgentActionType::StartRecording {
            playback_speed_multiplier,
            ..
        } => assert_eq!(playback_speed_multiplier, Some(1.5)),
        other => panic!("expected StartRecording, got {other:?}"),
    }
}

#[test]
fn start_recording_treats_real_time_playback_speed_as_unset() {
    for speed in [0.0, 1.0] {
        let mut tool_call = start_recording(None);
        tool_call.playback_speed = speed;
        let action =
            AIAgentActionType::try_from(tool_call).expect("valid start_recording should convert");
        match action {
            AIAgentActionType::StartRecording {
                playback_speed_multiplier,
                ..
            } => assert_eq!(
                playback_speed_multiplier, None,
                "speed {speed} should convert to None (real-time)"
            ),
            other => panic!("expected StartRecording, got {other:?}"),
        }
    }
}
