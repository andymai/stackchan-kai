//! Speech I/O surface of the entity.
//!
//! [`Voice`] is the entity's outbound audio interface. Today it carries
//! a single field, [`Voice::chirp_request`], that modifiers set when
//! they want the firmware to enqueue a one-shot audio clip (pickup
//! chirp, wake chirp, low-battery beep, camera-mode tones). The
//! firmware reads + clears this field after the modifier pass on each
//! frame.
//!
//! v2.x extensions sketch:
//! - `speech_queue`: bounded queue of phonemes / SSML / TTS-result
//!   audio buffers that the firmware's audio TX task drains. A
//!   `Skill` in [`Phase::Speech`] populates this from `mind.intent`.
//! - `is_speaking`: read-only flag the firmware sets while audio TX
//!   is actively playing speech, so other modifiers (e.g. `MouthOpenAudio`)
//!   can choose lip-sync vs envelope behaviour.
//!
//! The `chirp_request` design replaces the `pickup.just_fired() /
//! wake_on_voice.just_fired()` accessor pattern v0.x used: instead of
//! the firmware peeking modifier-internal state to gate audio enqueue,
//! modifiers publish *what they want* and the firmware decides *what
//! to do with it* — cleaner separation, mod-friendly.
//!
//! [`Phase::Speech`]: crate::app::Phase::Speech

/// One-shot audio clips the firmware can enqueue. Modifiers set
/// [`Voice::chirp_request`] to one of these; the render task reads it
/// after `App::run` returns and forwards to `audio::try_enqueue_*`.
///
/// Adding a new variant requires:
/// 1. A matching enqueue helper in `stackchan-firmware/src/audio.rs`
///    (e.g. `try_enqueue_my_chirp()`).
/// 2. The render task's chirp dispatch arm in `main.rs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ChirpKind {
    /// Pickup-detected chirp. Set by `PickupReaction` when an IMU
    /// pickup edge fires.
    Pickup,
    /// Voice-wake chirp. Set by `WakeOnVoice` when sustained audio
    /// activity wakes the entity from a non-attentive emotion.
    Wake,
    /// Low-battery alert beep. Set by `LowBatteryEmotion` on the
    /// downward-edge crossing of the enter threshold while unplugged.
    LowBatteryAlert,
    /// Camera-mode-enter tone. Set by the firmware's camera-mode
    /// toggle handler; not currently produced by a `Modifier`.
    CameraModeEnter,
    /// Camera-mode-exit tone. Counterpart to [`Self::CameraModeEnter`].
    CameraModeExit,
}

/// The entity's outbound audio surface.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Voice {
    /// One-shot chirp the firmware should enqueue this frame.
    /// `None` between frames; modifiers set this in their `update`.
    /// The render task reads + clears it after `App::run` returns.
    pub chirp_request: Option<ChirpKind>,
}
