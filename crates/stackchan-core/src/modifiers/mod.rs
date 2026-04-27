//! Reactive face / affect / motion modifiers.
//!
//! Each modifier implements the [`crate::Modifier`] trait and registers
//! with a [`crate::Director`] in a declared [`crate::director::Phase`].
//! The director sorts by `(phase, priority, registration_order)` and
//! ticks each modifier per frame.
//!
//! ## Phase population
//!
//! - **[`crate::director::Phase::Affect`]** — emotion deciders.
//!   Registered in this canonical order:
//!   1. [`EmotionFromTouch`] — consumes `entity.input.tap_pending`,
//!      advances `mind.affect.emotion`, sets
//!      `mind.autonomy.manual_until`.
//!   2. [`EmotionFromRemote`] — consumes `entity.input.remote_pending`,
//!      looks the `(address, command)` pair up in a user-supplied
//!      mapping table, sets emotion + autonomy.
//!   3. [`EmotionFromIntent`] — reads `mind.intent`, flips emotion to
//!      `Surprised` on `* → PickedUp` and to `Angry` on
//!      `* → Shaken`. Stands down when autonomy is already held.
//!      Driven by the [`crate::skills::Handling`] skill upstream.
//!   4. [`EmotionFromVoice`] — reads `perception.audio_rms`, flips to
//!      `Happy` on sustained voice. Wakes from `Sleepy`.
//!   5. [`IntentFromLoud`] — reads `perception.audio_rms`, flips to
//!      `Surprised` + writes `Intent::Startled` + queues
//!      `ChirpKind::Startle` on the rising edge above the loud
//!      threshold. Overrides `EmotionFromVoice` (sustained voice) but
//!      defers to explicit-input holds.
//!   6. [`EmotionFromAmbient`] — reads `perception.ambient_lux`, flips to
//!      `Sleepy` in dark rooms.
//!   7. [`EmotionFromBattery`] — reads `perception.battery_percent` and
//!      `perception.usb_power_present`, forces `Sleepy` below threshold
//!      while unplugged. Sets `voice.chirp_request` to `LowBatteryAlert`
//!      on the arming edge.
//!   8. [`EmotionCycle`] — autonomous emotion advancer. Stands
//!      down when `mind.autonomy.manual_until` is held.
//!
//! - **[`crate::director::Phase::Expression`]** — visual style:
//!   1. [`StyleFromEmotion`] — translates emotion into face style fields.
//!   2. [`StyleFromIntent`] — adds per-intent style overrides on top
//!      (cheek blush bump for `Petted`).
//!   3. [`GazeFromAttention`] — when `mind.attention` is `Tracking`,
//!      shifts both eye centers toward the target so eyes lead head
//!      motion.
//!   4. [`Blink`] — drives eye open/closed phase.
//!   5. [`Breath`] — vertical drift on all features.
//!   6. [`IdleDrift`] — occasional eye-center jitter.
//!
//! - **[`crate::director::Phase::Motion`]** — head motion:
//!   1. [`IdleHeadDrift`] — occasional brief head glances at
//!      randomised intervals, written to `motor.head_pose`.
//!   2. [`HeadFromEmotion`] — emotion-keyed pan/tilt bias added on top
//!      of head drift.
//!   3. [`HeadFromAttention`] — upward tilt bias when `mind.attention` is
//!      `Listening` (cocked-head listening posture). Added on top of
//!      head-drift + emotion bias.
//!   4. [`HeadFromIntent`] — brief asymmetric pan/tilt recoil on the
//!      entry edge into `Intent::Startled`. Fixed-duration impulse
//!      added on top of the other motion modifiers.
//!
//! - **[`crate::director::Phase::Audio`]** — audio-driven visual. 1
//!   modifier:
//!   1. [`MouthFromAudio`] — reads `perception.audio_rms`, writes
//!      `face.mouth.mouth_open`.
//!
//! - **[`crate::director::Phase::Cognition`]** — synthesis across
//!   percepts:
//!   1. [`AttentionFromTracking`] — reads `perception.tracking`,
//!      writes `mind.attention=Tracking{target}` after sustained
//!      camera motion; releases after the quiet window.
//!   2. [`RemoteCommandModifier`] — drains
//!      `entity.input.remote_command` set by an external control
//!      plane (firmware HTTP), holds emotion + autonomy
//!      ([`crate::RemoteCommand::SetEmotion`]) or attention
//!      ([`crate::RemoteCommand::LookAt`]) for a configurable
//!      window, and re-asserts each tick so cooperative cognition
//!      modifiers cannot stomp the operator's hold.
//!
//! Empty phases today (slots reserved for v2.x):
//! [`crate::director::Phase::Perception`],
//! [`crate::director::Phase::Speech`],
//! [`crate::director::Phase::Output`].

mod attention_from_tracking;
mod blink;
mod breath;
mod dormancy_from_activity;
mod emotion_cycle;
mod emotion_from_ambient;
mod emotion_from_battery;
mod emotion_from_intent;
mod emotion_from_remote;
mod emotion_from_touch;
mod emotion_from_voice;
mod gaze_from_attention;
mod head_from_attention;
mod head_from_emotion;
mod head_from_intent;
mod idle_drift;
mod idle_head_drift;
mod intent_from_body_touch;
mod intent_from_loud;
mod lost_target_search;
mod microsaccade_from_attention;
mod mouth_from_audio;
mod remote_command;
mod style_from_emotion;
mod style_from_intent;

pub use attention_from_tracking::{
    AttentionFromTracking, FACE_LOCK_HITS, FACE_RELEASE_MISSES, TRACKING_LOCK_TICKS,
    TRACKING_RELEASE_MS,
};
pub use blink::Blink;
pub use breath::Breath;
pub use dormancy_from_activity::{DORMANCY_TIMEOUT_MS, DormancyFromActivity};
pub use emotion_cycle::EmotionCycle;
pub use emotion_from_ambient::{
    AMBIENT_HOLD_MS, EmotionFromAmbient, SLEEPY_ENTER_LUX, SLEEPY_EXIT_LUX,
};
pub use emotion_from_battery::{
    EmotionFromBattery, LOW_BATTERY_ENTER_PERCENT, LOW_BATTERY_EXIT_PERCENT, LOW_BATTERY_HOLD_MS,
};
pub use emotion_from_intent::EmotionFromIntent;
pub use emotion_from_remote::{EmotionFromRemote, RemoteMapping};
pub use emotion_from_touch::{EMOTION_ORDER, EmotionFromTouch, MANUAL_HOLD_MS};
pub use emotion_from_voice::{
    EmotionFromVoice, WAKE_HOLD_MS, WAKE_RMS_THRESHOLD, WAKE_SUSTAIN_TICKS,
};
pub use gaze_from_attention::{GAZE_MAX_OFFSET_PX, GAZE_PIXELS_PER_DEG, GazeFromAttention};
pub use head_from_attention::{HeadFromAttention, LISTEN_HEAD_EASE_MS, LISTEN_HEAD_TILT_DEG};
pub use head_from_emotion::HeadFromEmotion;
pub use head_from_intent::{
    HeadFromIntent, STARTLE_HEAD_ATTACK_MS, STARTLE_HEAD_DECAY_MS, STARTLE_HEAD_PAN_DEG,
    STARTLE_HEAD_TILT_DEG, STARTLE_HEAD_TOTAL_MS,
};
pub use idle_drift::{
    DEFAULT_INTERVAL_MS as IDLE_DRIFT_INTERVAL_MS, DEFAULT_MAX_X as IDLE_DRIFT_MAX_X,
    DEFAULT_MAX_Y as IDLE_DRIFT_MAX_Y, IdleDrift,
};
pub use idle_head_drift::{
    GLANCE_EASE_IN_MS, GLANCE_EASE_OUT_MS, GLANCE_HOLD_MS, GLANCE_INTERVAL_MAX_MS,
    GLANCE_INTERVAL_MIN_MS, GLANCE_PAN_MAX_DEG, GLANCE_TILT_MAX_DEG, IdleHeadDrift,
};
pub use intent_from_body_touch::{
    BODY_GESTURE_HOLD_MS, DEFAULT_CENTRE_PRESS, DEFAULT_LEFT_PRESS, DEFAULT_RIGHT_PRESS,
    DEFAULT_SWIPE_BACKWARD, DEFAULT_SWIPE_FORWARD, GestureMapping, IntentFromBodyTouch,
    SWIPE_DELTA,
};
pub use intent_from_loud::{IntentFromLoud, STARTLE_HOLD_MS, STARTLE_RMS_THRESHOLD};
pub use lost_target_search::{
    LostTargetSearch, SEARCH_HOLD_MS, SEARCH_RETURN_MS, SEARCH_SACCADE_MS,
    SEARCH_SACCADE_OVERSHOOT, SEARCH_TOTAL_MS,
};
pub use microsaccade_from_attention::{
    MICROSACCADE_AMPLITUDE_PX, MICROSACCADE_DURATION_MS, MICROSACCADE_INTERVAL_MAX_MS,
    MICROSACCADE_INTERVAL_MIN_MS, MicrosaccadeFromAttention,
};
pub use mouth_from_audio::{
    DEFAULT_ATTACK_MS, DEFAULT_FULL_DB, DEFAULT_RELEASE_MS, DEFAULT_SILENCE_DB, MouthFromAudio,
};
pub use remote_command::RemoteCommandModifier;
pub use style_from_emotion::StyleFromEmotion;
pub use style_from_intent::{PETTING_BLUSH_BUMP, StyleFromIntent};
