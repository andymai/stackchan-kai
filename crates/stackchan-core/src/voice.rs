//! Speech I/O surface of the entity.
//!
//! [`Voice`] carries [`Voice::chirp_request`], which modifiers set
//! when they want the firmware to enqueue a one-shot audio clip
//! (pickup, wake, low-battery beep, camera-mode tone). The firmware
//! reads + clears the field after the modifier pass each frame.
//!
//! Modifiers publish what they want; the firmware decides how to play
//! it, so adding a new chirp kind doesn't require a modifier change
//! beyond the enum.

/// One-shot audio clips the firmware can enqueue.
///
/// Adding a variant requires:
/// 1. A matching enqueue helper in `stackchan-firmware/src/audio.rs`
///    (e.g. `try_enqueue_my_chirp()`).
/// 2. The render task's chirp dispatch arm in `main.rs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ChirpKind {
    /// Pickup-detected chirp. Set by `IntentReflex` on a transition
    /// into `Intent::PickedUp` (driven by the `Handling` skill).
    Pickup,
    /// Voice-wake chirp. Set by `WakeOnVoice` when sustained audio
    /// wakes the entity.
    Wake,
    /// Startle chirp. Set by `IntentFromLoud` on a transient acoustic
    /// spike (clap, shout, slam) — distinct from `Wake`, which is
    /// sustained-voice driven.
    Startle,
    /// Low-battery alert beep. Set by `LowBatteryEmotion` when the
    /// percent crosses the enter threshold while unplugged.
    LowBatteryAlert,
    /// Camera-mode-enter tone. Set by the firmware's camera-mode
    /// toggle handler.
    CameraModeEnter,
    /// Camera-mode-exit tone. Counterpart to [`Self::CameraModeEnter`].
    CameraModeExit,
}

/// The entity's outbound audio surface.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Voice {
    /// One-shot chirp the firmware should enqueue this frame. `None`
    /// between frames; modifiers set it in `update`; the render task
    /// reads + clears it after `Director::run` returns.
    pub chirp_request: Option<ChirpKind>,
}
