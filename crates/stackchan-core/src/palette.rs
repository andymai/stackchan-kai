//! Color palette — named presets that swap the avatar's base colours
//! at runtime.
//!
//! The palette layer affects the four "skin" colours of the avatar:
//! background, eye, mouth, and cheek. Decorator and bubble overlays
//! keep their own dedicated colours so a palette swap doesn't
//! desaturate or mute the symbolic-overlay layer's distinctness.
//!
//! Schema is intentionally a small enum of named presets rather than
//! free-form RGB565 fields. Named presets keep the operator surface
//! bounded (no need to validate arbitrary colour combinations against
//! "is the eye still readable?") and let future revisions grow the
//! palette set without breaking back-compat.
//!
//! ## Lifecycle
//!
//! `Face::palette` is populated at boot from the firmware's runtime
//! state, and updated on `POST /palette` / future MCP `set_palette`.
//! No expiry — the palette persists until the next operator change
//! (or the next reboot, since v0.2.0 doesn't yet persist the choice).

use embedded_graphics::pixelcolor::Rgb565;

/// Named palette preset. Picks one of the curated four-colour
/// combinations bundled with the firmware. Default `Default` matches
/// the original v0.x palette so a missing field on disk renders
/// identically.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Palette {
    /// Original Stack-chan palette: white background, black eyes,
    /// pink mouth + cheeks.
    #[default]
    Default,
    /// Dark mode: black background, white eyes, light pink for
    /// mouth + cheeks. Easier on the eyes in a dark room.
    Dark,
    /// "Cute" / uwu palette: light pink background, dark pink eyes,
    /// white mouth + cheeks. The avatar reads as a single
    /// blush-coloured blob with eyes.
    Cute,
    /// Dog-mode palette: tan background, brown eyes, pink mouth +
    /// cheeks. Pairs with the `m5stack-avatar` "dog face" geometry
    /// stylistically though kai retains the canonical face shape.
    Dog,
}

impl Palette {
    /// Lowercase wire name for the HTTP `/state` snapshot and the
    /// `POST /palette` body. Mirrors [`crate::Emotion::wire_str`]'s
    /// convention so a value lifted off `/state` can be posted back
    /// without case translation.
    #[must_use]
    pub const fn wire_str(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Dark => "dark",
            Self::Cute => "cute",
            Self::Dog => "dog",
        }
    }

    /// Single-byte wire encoding for any future BLE GATT exposure.
    /// Append-only; new variants get the next free integer.
    #[must_use]
    pub const fn wire_byte(self) -> u8 {
        match self {
            Self::Default => 0,
            Self::Dark => 1,
            Self::Cute => 2,
            Self::Dog => 3,
        }
    }

    /// Parse a lowercase wire string back into a [`Palette`].
    #[must_use]
    pub fn from_wire_str(s: &str) -> Option<Self> {
        match s {
            "default" => Some(Self::Default),
            "dark" => Some(Self::Dark),
            "cute" => Some(Self::Cute),
            "dog" => Some(Self::Dog),
            _ => None,
        }
    }

    /// Every variant in declaration order. Manually maintained — the
    /// exhaustive match arms in [`Self::wire_str`] / [`Self::wire_byte`]
    /// guard the encoding side, but this slice has no compile-time
    /// completeness check, so the `all_length_matches_variant_count`
    /// test is the trip-wire that forces an update on variant addition.
    pub const ALL: &'static [Self] = &[Self::Default, Self::Dark, Self::Cute, Self::Dog];

    /// Resolve to the four-colour palette for the renderer.
    #[must_use]
    pub const fn colors(self) -> PaletteColors {
        match self {
            Self::Default => PaletteColors {
                // White background, black eyes, the existing
                // `MOUTH_COLOR` (`#F58080` quantised) for both mouth
                // and cheeks.
                background: Rgb565::new(31, 63, 31),
                eye: Rgb565::new(0, 0, 0),
                mouth: Rgb565::new(30, 32, 16),
                cheek: Rgb565::new(30, 32, 16),
            },
            Self::Dark => PaletteColors {
                background: Rgb565::new(0, 0, 0),
                // High-contrast off-white for the eye so the
                // ellipse / arc reads as a single solid feature
                // against the black background.
                eye: Rgb565::new(28, 56, 28),
                mouth: Rgb565::new(31, 32, 20),
                cheek: Rgb565::new(31, 32, 20),
            },
            Self::Cute => PaletteColors {
                // Very light pink background — the avatar is a single
                // soft-pink blob.
                background: Rgb565::new(31, 50, 22),
                // Saturated pink for the eyes, contrasting against
                // the lighter background.
                eye: Rgb565::new(28, 16, 14),
                // Mouth stays white for the high-contrast smile.
                mouth: Rgb565::new(31, 63, 31),
                // Cheek is a medium pink between the background's
                // light tint and the eye's saturated pink — without
                // this the blush blends into the background at any
                // realistic `cheek_blush` weight and the cheek
                // disappears.
                cheek: Rgb565::new(30, 32, 16),
            },
            Self::Dog => PaletteColors {
                // Warm tan background.
                background: Rgb565::new(28, 48, 14),
                // Brown eyes.
                eye: Rgb565::new(10, 14, 4),
                mouth: Rgb565::new(30, 32, 16),
                cheek: Rgb565::new(30, 32, 16),
            },
        }
    }
}

/// Resolved palette — the four colours the renderer uses for the
/// avatar's skin. Returned by [`Palette::colors`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PaletteColors {
    /// Background-clear colour (the canvas is filled with this before
    /// drawing eyes / mouth / cheeks / overlays).
    pub background: Rgb565,
    /// Eye colour — the filled ellipse for `eye_curve == 0` or the
    /// stroked polyline arc for non-zero curve.
    pub eye: Rgb565,
    /// Mouth colour — line / ellipse / arc, all the same colour.
    pub mouth: Rgb565,
    /// Cheek-blush circle colour. Often the same as `mouth`.
    pub cheek: Rgb565,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_length_matches_variant_count() {
        assert_eq!(
            Palette::ALL.len(),
            4,
            "update Palette::ALL when adding a variant"
        );
    }

    #[test]
    fn wire_byte_mapping_is_stable() {
        assert_eq!(Palette::Default.wire_byte(), 0);
        assert_eq!(Palette::Dark.wire_byte(), 1);
        assert_eq!(Palette::Cute.wire_byte(), 2);
        assert_eq!(Palette::Dog.wire_byte(), 3);
    }

    #[test]
    fn wire_str_round_trip_for_all_variants() {
        for &p in Palette::ALL {
            let s = p.wire_str();
            assert_eq!(Palette::from_wire_str(s), Some(p));
        }
    }

    #[test]
    fn from_wire_str_rejects_unknown() {
        assert_eq!(Palette::from_wire_str(""), None);
        assert_eq!(Palette::from_wire_str("DEFAULT"), None); // case sensitive
        assert_eq!(Palette::from_wire_str("rainbow"), None);
    }

    #[test]
    fn each_palette_renders_distinct_background() {
        // Sanity guard against two palettes silently sharing the
        // background colour and reading as the same theme on screen.
        for (i, &a) in Palette::ALL.iter().enumerate() {
            for &b in &Palette::ALL[i + 1..] {
                assert_ne!(
                    a.colors().background,
                    b.colors().background,
                    "{a:?} and {b:?} share a background colour",
                );
            }
        }
    }

    #[test]
    fn default_palette_matches_original_constants() {
        // Pin the Default palette to the legacy hardcoded values so
        // the avatar's pre-PR-#8 look survives the refactor exactly.
        let c = Palette::Default.colors();
        assert_eq!(c.background, Rgb565::new(31, 63, 31)); // white
        assert_eq!(c.eye, Rgb565::new(0, 0, 0)); // black
        assert_eq!(c.mouth, Rgb565::new(30, 32, 16)); // legacy MOUTH_COLOR
        assert_eq!(c.cheek, Rgb565::new(30, 32, 16));
    }
}
