//! Named one-shot motion presets.
//!
//! [`NamedMotion`] catalogs a small set of canonical avatar
//! gestures (greet, nod, shake, laugh) and exposes a compile-time
//! [`crate::DanceScript`] for each. The firmware HTTP / MCP layer
//! looks the variant up by wire name and routes the resulting
//! script through the existing dance-player path, so no separate
//! "named-motion player" modifier is needed — the `DancePlayer`
//! already handles arbitrary keyframe sequences.
//!
//! ## Why baked scripts vs free choreography
//!
//! Operators want a one-tap "say hi" without having to write a
//! JSON keyframe sequence. The variants here mirror the upstream
//! Arduino library's `motion(Motion)` API: greeting, nod, head
//! shake, laugh. Each is a short (≤ 2 s) preset that returns the
//! head to its baseline pose on exit so a follow-up emotion or
//! tracking command starts from a known state.

use alloc::vec;

use crate::dance::{DanceScript, Keyframe};
use crate::decorator::Decorator;
use crate::emotion::Emotion;

/// A canonical one-shot motion preset.
///
/// Picked by the HTTP `POST /motion` route and the MCP
/// `play_motion` tool. Maps to a compile-time [`DanceScript`] via
/// [`Self::script`]; the firmware writes the result into
/// `entity.input.dance_script` so the existing dance player drives
/// the gesture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum NamedMotion {
    /// "Hello" — slight upward tilt + Happy emotion for ~1 second.
    Greet,
    /// "Yes" — two short downward nods.
    Nod,
    /// "No" — left-right pan oscillation.
    Shake,
    /// "Haha" — rapid head bobbing with Heart decorator + Loved
    /// emotion.
    Laugh,
}

impl NamedMotion {
    /// Every variant, in declaration order. Maintained alongside
    /// the wire-encoding match arms so a future variant is
    /// caught by the
    /// `all_length_matches_variant_count` test.
    pub const ALL: &'static [Self] = &[Self::Greet, Self::Nod, Self::Shake, Self::Laugh];

    /// Lowercase wire name for the HTTP body / MCP argument /
    /// future `/state` snapshot.
    #[must_use]
    pub const fn wire_str(self) -> &'static str {
        match self {
            Self::Greet => "greet",
            Self::Nod => "nod",
            Self::Shake => "shake",
            Self::Laugh => "laugh",
        }
    }

    /// Parse a lowercase wire string back into a [`NamedMotion`].
    #[must_use]
    pub fn from_wire_str(s: &str) -> Option<Self> {
        match s {
            "greet" => Some(Self::Greet),
            "nod" => Some(Self::Nod),
            "shake" => Some(Self::Shake),
            "laugh" => Some(Self::Laugh),
            _ => None,
        }
    }

    /// Compile-time keyframe sequence for this motion. Allocates a
    /// fresh [`DanceScript`] on each call — cheap (≤ 6 keyframes)
    /// and called only on the operator-triggered edge.
    #[must_use]
    pub fn script(self) -> DanceScript {
        match self {
            Self::Greet => greet_script(),
            Self::Nod => nod_script(),
            Self::Shake => shake_script(),
            Self::Laugh => laugh_script(),
        }
    }
}

/// "Hello" tilt-up + Happy. ~1.1 s.
fn greet_script() -> DanceScript {
    DanceScript {
        keyframes: vec![
            Keyframe {
                at_ms: 0,
                tilt_deg: Some(0.0),
                emotion: Some(Emotion::Happy),
                ..Keyframe::default()
            },
            Keyframe {
                at_ms: 200,
                tilt_deg: Some(-10.0),
                ..Keyframe::default()
            },
            Keyframe {
                at_ms: 800,
                tilt_deg: Some(-10.0),
                ..Keyframe::default()
            },
            Keyframe {
                at_ms: 1100,
                tilt_deg: Some(0.0),
                ..Keyframe::default()
            },
        ],
    }
}

/// "Yes" two-beat tilt-down nod. ~0.7 s.
fn nod_script() -> DanceScript {
    DanceScript {
        keyframes: vec![
            Keyframe {
                at_ms: 0,
                tilt_deg: Some(0.0),
                ..Keyframe::default()
            },
            Keyframe {
                at_ms: 180,
                tilt_deg: Some(12.0),
                ..Keyframe::default()
            },
            Keyframe {
                at_ms: 360,
                tilt_deg: Some(0.0),
                ..Keyframe::default()
            },
            Keyframe {
                at_ms: 540,
                tilt_deg: Some(12.0),
                ..Keyframe::default()
            },
            Keyframe {
                at_ms: 720,
                tilt_deg: Some(0.0),
                ..Keyframe::default()
            },
        ],
    }
}

/// "No" left-right pan oscillation. ~0.8 s.
fn shake_script() -> DanceScript {
    DanceScript {
        keyframes: vec![
            Keyframe {
                at_ms: 0,
                pan_deg: Some(0.0),
                ..Keyframe::default()
            },
            Keyframe {
                at_ms: 150,
                pan_deg: Some(15.0),
                ..Keyframe::default()
            },
            Keyframe {
                at_ms: 300,
                pan_deg: Some(-15.0),
                ..Keyframe::default()
            },
            Keyframe {
                at_ms: 450,
                pan_deg: Some(15.0),
                ..Keyframe::default()
            },
            Keyframe {
                at_ms: 600,
                pan_deg: Some(-15.0),
                ..Keyframe::default()
            },
            Keyframe {
                at_ms: 800,
                pan_deg: Some(0.0),
                ..Keyframe::default()
            },
        ],
    }
}

/// "Haha" rapid bobbing + Heart + Loved. ~0.85 s.
fn laugh_script() -> DanceScript {
    DanceScript {
        keyframes: vec![
            Keyframe {
                at_ms: 0,
                tilt_deg: Some(0.0),
                emotion: Some(Emotion::Loved),
                decorator: Some(Decorator::Heart),
                ..Keyframe::default()
            },
            Keyframe {
                at_ms: 150,
                tilt_deg: Some(6.0),
                ..Keyframe::default()
            },
            Keyframe {
                at_ms: 300,
                tilt_deg: Some(-3.0),
                ..Keyframe::default()
            },
            Keyframe {
                at_ms: 450,
                tilt_deg: Some(6.0),
                ..Keyframe::default()
            },
            Keyframe {
                at_ms: 600,
                tilt_deg: Some(-3.0),
                ..Keyframe::default()
            },
            Keyframe {
                at_ms: 850,
                tilt_deg: Some(0.0),
                ..Keyframe::default()
            },
        ],
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    reason = "test-only: Option::unwrap on values just constructed in the helper above"
)]
mod tests {
    use super::*;

    #[test]
    fn all_length_matches_variant_count() {
        assert_eq!(
            NamedMotion::ALL.len(),
            4,
            "update NamedMotion::ALL when adding a variant",
        );
    }

    #[test]
    fn wire_str_round_trip() {
        for &m in NamedMotion::ALL {
            assert_eq!(NamedMotion::from_wire_str(m.wire_str()), Some(m));
        }
    }

    #[test]
    fn from_wire_str_rejects_unknown() {
        assert_eq!(NamedMotion::from_wire_str(""), None);
        assert_eq!(NamedMotion::from_wire_str("GREET"), None);
        assert_eq!(NamedMotion::from_wire_str("wave"), None);
    }

    #[test]
    fn every_motion_starts_at_zero_and_returns_to_baseline() {
        for &m in NamedMotion::ALL {
            let s = m.script();
            assert!(!s.keyframes.is_empty());
            assert_eq!(s.keyframes[0].at_ms, 0, "{m:?} must start at t=0");
            let last = s.keyframes.last().unwrap();
            let pan = last.pan_deg.unwrap_or(0.0);
            let tilt = last.tilt_deg.unwrap_or(0.0);
            assert!(
                pan.abs() <= 0.01,
                "{m:?} ends at pan={pan} (must be ~0 so the next command starts from baseline)"
            );
            assert!(tilt.abs() <= 0.01, "{m:?} ends at tilt={tilt} (must be ~0)");
        }
    }
}
