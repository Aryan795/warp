mod telemetry;
mod view;

pub use telemetry::FeatureIntroModalTelemetryEvent;
pub use view::{
    FEATURE_INTROS, FeatureIntro, FeatureIntroCtaTarget, FeatureIntroId, FeatureIntroModal,
    FeatureIntroModalEvent, feature_intro_by_id, init, with_email_id_prefill,
};
