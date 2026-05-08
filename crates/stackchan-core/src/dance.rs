//! Dance choreography schema.
//!
//! [`DanceScript`] is a sequence of [`Keyframe`]s sampled by
//! [`crate::modifiers::DancePlayer`] each tick. Each keyframe is
//! anchored at a millisecond offset from the script's start and
//! carries an optional value for any of three channels:
//!
//! - **Motion** — `pan_deg` / `tilt_deg`.
//! - **Avatar** — `emotion` / `decorator`.
//! - **RGB**    — `r` / `g` / `b` LED-ring colour.
//!
//! Channels are sampled independently: a keyframe that only sets
//! `pan_deg` leaves the avatar and RGB channels alone, and the
//! player keeps using whatever the most-recent matching keyframe
//! supplied for those channels.
//!
//! ## Crate split
//!
//! Schema + validation live here so the player modifier (also in
//! this crate) can consume them without inverting the dependency
//! direction. JSON parsing lives in `stackchan-net::dance` —
//! that's the wire-format crate.

use alloc::vec::Vec;

use crate::decorator::Decorator;
use crate::emotion::Emotion;

/// Maximum number of keyframes a single script can carry.
///
/// Sized so a 30-second dance at 100 ms per keyframe (300 frames)
/// fits comfortably with margin. Caps the worst-case allocation a
/// malformed script can request.
pub const MAX_KEYFRAMES: usize = 1024;

/// How long after the last keyframe the player keeps holding overrides
/// before clearing them. 200 ms gives the audience time to register
/// the final pose without dragging into the next emotion cycle.
pub const SCRIPT_TAIL_MS: u32 = 200;

/// Minimum spacing between successive keyframe `at_ms` values.
///
/// 0 ms (multiple frames at the same instant) is allowed — different
/// channels can change simultaneously without forcing one to lag.
/// Negative spacing (out-of-order keyframes) is rejected.
pub const MIN_KEYFRAME_SPACING_MS: u32 = 0;

/// One keyframe in a [`DanceScript`].
///
/// Every field after `at_ms` is optional; the player carries the
/// most-recent value forward for unset fields. `Eq` deliberately
/// withheld because the float fields make value-equality on the
/// raw struct a footgun (NaN ≠ NaN).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Keyframe {
    /// Offset from the script's start, in milliseconds.
    pub at_ms: u32,
    /// Pan target in degrees. Player passes through to
    /// `motor.head_pose.pan_deg` (clamped by the head driver).
    pub pan_deg: Option<f32>,
    /// Tilt target in degrees. Player passes through to
    /// `motor.head_pose.tilt_deg`.
    pub tilt_deg: Option<f32>,
    /// Emotion override. Player writes `mind.affect.emotion` and
    /// holds autonomy through the dance window.
    pub emotion: Option<Emotion>,
    /// Decorator override. Player writes `face.decorator` until the
    /// next keyframe with a different decorator (or until the script
    /// ends).
    pub decorator: Option<Decorator>,
    /// LED-ring red component. RGB channel is sampled as a triple —
    /// when any of `r`/`g`/`b` is set on a keyframe all three must be.
    pub r: Option<u8>,
    /// LED-ring green component.
    pub g: Option<u8>,
    /// LED-ring blue component.
    pub b: Option<u8>,
}

impl Keyframe {
    /// `(r, g, b)` triple if all three RGB components are set; `None`
    /// otherwise (including the partial-triple authoring error case
    /// — [`validate`] rejects partial triples up front so the player
    /// only sees fully-set or fully-unset RGB samples).
    #[must_use]
    pub const fn rgb(&self) -> Option<(u8, u8, u8)> {
        match (self.r, self.g, self.b) {
            (Some(r), Some(g), Some(b)) => Some((r, g, b)),
            _ => None,
        }
    }
}

/// A complete dance script.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DanceScript {
    /// Keyframes in `at_ms` order. [`validate`] enforces the ordering;
    /// the player relies on it for the linear-scan sampler.
    pub keyframes: Vec<Keyframe>,
}

/// Reasons [`validate`] rejects a script. Callers (the JSON parser
/// in `stackchan-net::dance`, future RON loaders, BLE keyframe
/// streams) translate these into their own error surfaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DanceError {
    /// `keyframes` is empty.
    Empty,
    /// `keyframes` exceeds [`MAX_KEYFRAMES`].
    TooManyKeyframes,
    /// A keyframe's `at_ms` is less than the previous keyframe's
    /// (script must be in time order).
    OutOfOrder,
    /// A keyframe sets some but not all of `r` / `g` / `b`. RGB
    /// channel writes must be the full triple.
    PartialRgb,
}

/// Validate a parsed script.
///
/// # Errors
///
/// Returns the [`DanceError`] variant identifying the structural
/// failure.
pub fn validate(script: &DanceScript) -> Result<(), DanceError> {
    if script.keyframes.is_empty() {
        return Err(DanceError::Empty);
    }
    if script.keyframes.len() > MAX_KEYFRAMES {
        return Err(DanceError::TooManyKeyframes);
    }
    let mut prev_at: Option<u32> = None;
    for kf in &script.keyframes {
        if let Some(prev) = prev_at
            && kf.at_ms < prev + MIN_KEYFRAME_SPACING_MS
        {
            return Err(DanceError::OutOfOrder);
        }
        prev_at = Some(kf.at_ms);
        let any_rgb = kf.r.is_some() || kf.g.is_some() || kf.b.is_some();
        let all_rgb = kf.r.is_some() && kf.g.is_some() && kf.b.is_some();
        if any_rgb && !all_rgb {
            return Err(DanceError::PartialRgb);
        }
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "test-only assertions on Some values")]
mod tests {
    use super::*;
    use alloc::vec;

    fn kf(at_ms: u32) -> Keyframe {
        Keyframe {
            at_ms,
            ..Keyframe::default()
        }
    }

    #[test]
    fn keyframe_rgb_returns_triple_when_all_set() {
        let frame = Keyframe {
            r: Some(10),
            g: Some(20),
            b: Some(30),
            ..Keyframe::default()
        };
        assert_eq!(frame.rgb(), Some((10, 20, 30)));
    }

    #[test]
    fn keyframe_rgb_returns_none_when_none_set() {
        assert_eq!(Keyframe::default().rgb(), None);
    }

    #[test]
    fn keyframe_rgb_returns_none_for_partial_triple() {
        let frame = Keyframe {
            r: Some(10),
            g: Some(20),
            b: None,
            ..Keyframe::default()
        };
        assert_eq!(frame.rgb(), None);
    }

    #[test]
    fn validate_rejects_empty_script() {
        assert_eq!(
            validate(&DanceScript {
                keyframes: Vec::new()
            }),
            Err(DanceError::Empty)
        );
    }

    #[test]
    fn validate_rejects_partial_rgb_triple() {
        let script = DanceScript {
            keyframes: vec![Keyframe {
                at_ms: 0,
                r: Some(10),
                g: Some(20),
                b: None,
                ..Keyframe::default()
            }],
        };
        assert_eq!(validate(&script), Err(DanceError::PartialRgb));
    }

    #[test]
    fn validate_rejects_out_of_order() {
        let script = DanceScript {
            keyframes: vec![kf(100), kf(50)],
        };
        assert_eq!(validate(&script), Err(DanceError::OutOfOrder));
    }

    #[test]
    fn validate_accepts_simultaneous_keyframes() {
        let script = DanceScript {
            keyframes: vec![kf(0), kf(0), kf(100)],
        };
        assert!(validate(&script).is_ok());
    }

    #[test]
    fn validate_accepts_in_order_script() {
        let script = DanceScript {
            keyframes: vec![kf(0), kf(500), kf(1000)],
        };
        assert!(validate(&script).is_ok());
    }
}
