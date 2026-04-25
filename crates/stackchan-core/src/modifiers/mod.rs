//! Animation modifiers — the v0.x reactive face/affect/motion behaviors.
//!
//! Each modifier in this module implements the [`crate::Modifier`]
//! trait and registers with a [`crate::Director`] in a declared
//! [`crate::director::Phase`]. The director sorts by `(phase, priority,
//! registration order)` and ticks each modifier per frame.
//!
//! ## Phase population (today's 14 modifiers)
//!
//! - **[`crate::director::Phase::Affect`]** — emotion deciders. 7
//!   modifiers, registered in this canonical order:
//!   1. [`EmotionTouch`] — consumes `entity.input.tap_pending`,
//!      advances `mind.affect.emotion`, sets
//!      `mind.autonomy.manual_until`.
//!   2. [`RemoteCommand`] — consumes `entity.input.remote_pending`,
//!      looks the `(address, command)` pair up in a user-supplied
//!      mapping table, sets emotion + autonomy.
//!   3. [`PickupReaction`] — reads `perception.accel_g`, flips
//!      emotion to `Surprised` on detected lifts/drops. Stands
//!      down when autonomy is already held.
//!   4. [`WakeOnVoice`] — reads `perception.audio_rms`, flips to
//!      `Happy` on sustained voice. Wakes from `Sleepy`.
//!   5. [`AmbientSleepy`] — reads `perception.ambient_lux`, flips to
//!      `Sleepy` in dark rooms.
//!   6. [`LowBatteryEmotion`] — reads `perception.battery_percent` and
//!      `perception.usb_power_present`, forces `Sleepy` below threshold
//!      while unplugged. Sets `voice.chirp_request` to `LowBatteryAlert`
//!      on the arming edge.
//!   7. [`EmotionCycle`] — autonomous emotion advancer. Stands
//!      down when `mind.autonomy.manual_until` is held.
//!
//! - **[`crate::director::Phase::Expression`]** — visual style. 4
//!   modifiers:
//!   1. [`EmotionStyle`] — translates emotion into face style fields.
//!   2. [`Blink`] — drives eye open/closed phase.
//!   3. [`Breath`] — vertical drift on all features.
//!   4. [`IdleDrift`] — occasional eye-center jitter.
//!
//! - **[`crate::director::Phase::Motion`]** — head motion. 2 modifiers:
//!   1. [`IdleSway`] — slow pan/tilt head wander written to
//!      `motor.head_pose`.
//!   2. [`EmotionHead`] — emotion-keyed pan/tilt bias added on top
//!      of sway.
//!
//! - **[`crate::director::Phase::Audio`]** — audio-driven visual. 1
//!   modifier:
//!   1. [`MouthOpenAudio`] — reads `perception.audio_rms`, writes
//!      `face.mouth.mouth_open`.
//!
//! Empty phases today (slots reserved for v2.x):
//! [`crate::director::Phase::Perception`],
//! [`crate::director::Phase::Cognition`],
//! [`crate::director::Phase::Speech`],
//! [`crate::director::Phase::Output`].

mod ambient_sleepy;
mod blink;
mod breath;
mod emotion_cycle;
mod emotion_head;
mod emotion_style;
mod emotion_touch;
mod idle_drift;
mod idle_sway;
mod low_battery;
mod mouth_open_audio;
mod pickup_reaction;
mod remote_command;
mod wake_on_voice;

pub use ambient_sleepy::{AMBIENT_HOLD_MS, AmbientSleepy, SLEEPY_ENTER_LUX, SLEEPY_EXIT_LUX};
pub use blink::Blink;
pub use breath::Breath;
pub use emotion_cycle::EmotionCycle;
pub use emotion_head::EmotionHead;
pub use emotion_style::EmotionStyle;
pub use emotion_touch::{EMOTION_ORDER, EmotionTouch, MANUAL_HOLD_MS};
pub use idle_drift::IdleDrift;
pub use idle_sway::IdleSway;
pub use low_battery::{
    LOW_BATTERY_ENTER_PERCENT, LOW_BATTERY_EXIT_PERCENT, LOW_BATTERY_HOLD_MS, LowBatteryEmotion,
};
pub use mouth_open_audio::{
    DEFAULT_ATTACK_MS, DEFAULT_FULL_DB, DEFAULT_RELEASE_MS, DEFAULT_SILENCE_DB, MouthOpenAudio,
};
pub use pickup_reaction::{PICKUP_DEBOUNCE_MS, PICKUP_DEVIATION_G, PickupReaction};
pub use remote_command::{RemoteCommand, RemoteMapping};
pub use wake_on_voice::{WAKE_HOLD_MS, WAKE_RMS_THRESHOLD, WAKE_SUSTAIN_TICKS, WakeOnVoice};
