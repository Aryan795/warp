use warpui_core::App;

use super::{
    MAX_RESAMPLED_SAMPLES, StartListeningError, VoiceInput, VoiceInputLifecycle,
    VoiceInputLifecycleState, VoiceInputState, VoiceInputToggledFrom, append_resampled_samples,
};

#[test]
fn lifecycle_rejects_overlapping_sessions() {
    let mut lifecycle = VoiceInputLifecycle::default();
    assert!(lifecycle.start());

    assert_eq!(lifecycle.state(), VoiceInputLifecycleState::Listening);
    assert!(!lifecycle.start());
    assert!(lifecycle.begin_transcribing());
    assert_eq!(lifecycle.state(), VoiceInputLifecycleState::Transcribing);
    assert!(!lifecycle.start());
}

#[test]
fn lifecycle_rejects_invalid_transitions() {
    let mut lifecycle = VoiceInputLifecycle::default();
    assert!(!lifecycle.begin_transcribing());
    assert!(!lifecycle.complete());
    assert!(!lifecycle.fail());
    assert!(lifecycle.start());
    assert!(!lifecycle.complete());
    assert_eq!(lifecycle.state(), VoiceInputLifecycleState::Listening);
}

#[test]
fn lifecycle_cancellation_returns_to_idle() {
    let mut lifecycle = VoiceInputLifecycle::default();
    assert!(lifecycle.start());
    assert!(lifecycle.begin_transcribing());
    assert!(lifecycle.cancel());

    assert_eq!(lifecycle.state(), VoiceInputLifecycleState::Idle);
    assert!(!lifecycle.complete());
    assert!(!lifecycle.fail());
    assert!(!lifecycle.cancel());
}

#[test]
fn recorder_rejects_a_new_session_while_transcribing() {
    App::test((), |mut app| async move {
        let voice_input = app.add_model(VoiceInput::new);
        voice_input.update(&mut app, |voice_input, ctx| {
            voice_input.state = VoiceInputState::Transcribing;
            assert!(matches!(
                voice_input.start_listening(ctx, VoiceInputToggledFrom::Button),
                Err(StartListeningError::AlreadyRunning)
            ));
        });
    });
}

#[test]
fn append_resampled_samples_accumulates_normally_below_the_cap() {
    let mut buffer = vec![];

    let at_cap = append_resampled_samples(&mut buffer, &[1.0, 2.0, 3.0]);

    assert_eq!(buffer, vec![1.0, 2.0, 3.0]);
    assert!(!at_cap);
}

#[test]
fn append_resampled_samples_stops_growing_at_the_cap() {
    let mut buffer = vec![0.0; MAX_RESAMPLED_SAMPLES - 2];

    let at_cap = append_resampled_samples(&mut buffer, &[1.0, 2.0, 3.0, 4.0]);

    assert_eq!(buffer.len(), MAX_RESAMPLED_SAMPLES);
    assert!(at_cap);

    // Once at capacity, further appends are no-ops rather than growing the buffer.
    let still_at_cap = append_resampled_samples(&mut buffer, &[5.0, 6.0]);
    assert_eq!(buffer.len(), MAX_RESAMPLED_SAMPLES);
    assert!(still_at_cap);
}

#[test]
fn append_resampled_samples_retains_already_captured_audio_when_capped() {
    let mut buffer = vec![0.0; MAX_RESAMPLED_SAMPLES - 2];
    buffer[MAX_RESAMPLED_SAMPLES - 3] = 42.0;

    append_resampled_samples(&mut buffer, &[1.0, 2.0, 3.0]);

    // The samples captured before the cap are untouched, and the truncated tail of
    // the final chunk was appended rather than the whole chunk being dropped.
    assert_eq!(buffer[MAX_RESAMPLED_SAMPLES - 3], 42.0);
    assert_eq!(buffer[MAX_RESAMPLED_SAMPLES - 2], 1.0);
    assert_eq!(buffer[MAX_RESAMPLED_SAMPLES - 1], 2.0);
}
