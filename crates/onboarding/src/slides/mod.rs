// twarp: `agent_slide` and its private helper `two_line_button` were deleted
// in 2c-a; the surviving data types they exposed live in `agent_data_types`
// and are still consumed by the onboarding state model and the post-
// onboarding settings application. 2c-d / 2c-e remove these data types
// alongside `app/src/ai/onboarding.rs` and `crates/ai/`.
mod agent_data_types;
mod bottom_nav;
mod customize_slide;
mod free_user_no_ai_slide;
mod intention_slide;
mod intro_slide;
pub mod layout;
mod onboarding_slide;
mod progress_dots;
mod project_slide;
pub mod slide_content;
mod theme_picker_slide;
mod third_party_slide;
mod toggle_card;

pub use agent_data_types::{AgentAutonomy, AgentDevelopmentSettings, OnboardingModelInfo};
pub use bottom_nav::onboarding_bottom_nav;
pub use customize_slide::CustomizeUISlide;
pub use free_user_no_ai_slide::FreeUserNoAiSlide;
pub use intention_slide::IntentionSlide;
pub use intro_slide::{IntroSlide, IntroSlideEvent};
pub use onboarding_slide::OnboardingSlide;
pub use project_slide::{ProjectOnboardingSettings, ProjectSlide};
pub use theme_picker_slide::{ThemePickerSlide, ThemePickerSlideEvent};
pub use third_party_slide::ThirdPartySlide;
