//! [`Soliloquy`] — opt-in autonomous bubble beat.
//!
//! Writes a randomly picked phrase to [`crate::bubble::BubbleState`]
//! at random intervals when nothing else is engaging the avatar.
//!
//! Disabled by default. Operators flip
//! `stackchan_net::BehaviorConfig::soliloquy_enabled` to opt in;
//! the firmware passes that flag to [`Soliloquy::with_enabled`] at
//! construction.
//!
//! ## Why a modifier (not a skill)
//!
//! Skills require a `should_fire` boolean that the director polls. The
//! soliloquy beat is purely time-driven: every tick the modifier
//! checks `now >= next_fire_at` and either fires (write bubble +
//! re-roll the next deadline) or no-ops. That fits the modifier
//! contract more naturally than a skill — and lets us reuse the
//! same xorshift PRNG / random-interval shape as
//! [`super::IdleHeadDrift`].
//!
//! ## Bubble-only for v0.2.0
//!
//! Audio playback is intentionally not wired here. The baked phrase
//! catalog ([`crate::voice::PhraseId`]) doesn't yet carry general
//! soliloquy lines — adding the audio path means generating PCM
//! assets, deciding on TX priority, etc. The bubble line alone gets
//! the visible feature in front of operators while the audio
//! follow-up matures.
//!
//! ## Phase + priority
//!
//! Runs in [`Phase::Decoration`] at priority `0` (between
//! [`super::BubbleExpiry`] at `-10` and the decorator triggers at
//! `0+`). The director sorts ties by registration order, so as long as
//! [`super::BubbleExpiry`] is registered first the expired bubble is
//! cleared before this modifier checks the field — meaning a stale
//! expired bubble doesn't suppress a fresh fire.

use core::num::NonZeroU32;

use crate::bubble::BubbleState;
use crate::clock::Instant;
use crate::director::{Field, ModifierMeta, Phase};
use crate::entity::Entity;
use crate::modifier::Modifier;

/// Phrase pool — short observational lines the avatar mumbles.
///
/// Kept short (≤ 28 ASCII chars to fit in the bubble at
/// `FONT_10X20`) and non-prescriptive — the avatar mumbling these is
/// the joke; the audience shouldn't read them as instructions.
pub const SOLILOQUY_LINES: &[&str] = &[
    "hmm...",
    "I wonder...",
    "interesting.",
    "what was that?",
    "...",
    "hello?",
    "let's see.",
    "oh!",
    "huh.",
    "okay.",
];

/// Minimum interval between soliloquy fires, in ms. `30_000` reads as
/// "occasional ambient mumble" rather than "narrating constantly."
pub const SOLILOQUY_INTERVAL_MIN_MS: u64 = 30_000;
/// Maximum interval between soliloquy fires, in ms.
pub const SOLILOQUY_INTERVAL_MAX_MS: u64 = 90_000;
/// How long each soliloquy bubble stays on screen, in ms. Long enough
/// for the audience to read 1–2 short words; short enough that the
/// face isn't occluded for any meaningful interaction window.
pub const SOLILOQUY_BUBBLE_TTL_MS: u64 = 4_000;

/// Default xorshift32 seed. Different from the seeds in
/// [`super::IdleDrift`] / [`super::IdleHeadDrift`] so the schedules
/// don't synchronise out of the box.
#[allow(
    clippy::unwrap_used,
    reason = "const-evaluated against a non-zero literal: unwrap can't fire at runtime"
)]
const DEFAULT_SEED: NonZeroU32 = NonZeroU32::new(0xDEAD_BEEF).unwrap();

/// Modifier that fires soliloquy bubbles at random intervals.
#[derive(Debug, Clone, Copy)]
pub struct Soliloquy {
    /// `true` iff the firmware-side config has soliloquy turned on.
    /// `false` short-circuits `update` to a no-op.
    enabled: bool,
    /// xorshift32 PRNG state. Same algorithm as
    /// [`super::IdleHeadDrift::next_u32`].
    rng_state: u32,
    /// Wall-clock instant at which the next soliloquy fires. `None`
    /// until the first tick anchors the schedule.
    next_fire_at: Option<Instant>,
}

impl Soliloquy {
    /// Construct enabled with a custom seed. Test helper.
    #[must_use]
    pub const fn with_seed(enabled: bool, seed: NonZeroU32) -> Self {
        Self {
            enabled,
            rng_state: seed.get(),
            next_fire_at: None,
        }
    }

    /// Construct with the default seed. Pass the operator's
    /// `behavior.soliloquy_enabled` flag from
    /// `stackchan_net::BehaviorConfig`.
    #[must_use]
    pub const fn with_enabled(enabled: bool) -> Self {
        Self::with_seed(enabled, DEFAULT_SEED)
    }

    /// Construct disabled. Equivalent to `with_enabled(false)` —
    /// makes the no-op intent explicit when the firmware always
    /// constructs the modifier and only flips the flag at runtime.
    #[must_use]
    pub const fn new() -> Self {
        Self::with_enabled(false)
    }

    /// Advance the xorshift32 state and return the next pseudo-random
    /// `u32`.
    const fn next_u32(&mut self) -> u32 {
        let mut x = self.rng_state;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng_state = x;
        x
    }

    /// Pick a uniform `u64` in `[lo, hi]` from the next PRNG draw.
    fn rand_interval(&mut self, lo: u64, hi: u64) -> u64 {
        if hi <= lo {
            return lo;
        }
        let span = hi - lo + 1;
        let draw = u64::from(self.next_u32()) % span;
        lo + draw
    }
}

impl Default for Soliloquy {
    fn default() -> Self {
        Self::new()
    }
}

impl Modifier for Soliloquy {
    fn meta(&self) -> &'static ModifierMeta {
        static META: ModifierMeta = ModifierMeta {
            name: "Soliloquy",
            description: "Writes a random short phrase from SOLILOQUY_LINES to face.bubble \
                          at randomised 30-90 s intervals when enabled. Bubble-only for \
                          this iteration; audio integration ships later. No-op when \
                          BehaviorConfig::soliloquy_enabled is false.",
            phase: Phase::Decoration,
            priority: 0,
            reads: &[Field::Bubble],
            writes: &[Field::Bubble],
        };
        &META
    }

    fn update(&mut self, entity: &mut Entity) {
        if !self.enabled {
            return;
        }
        let now = entity.tick.now;

        // Anchor the schedule on the first wakeful tick. Done in two
        // steps because the closure form clashes with the second
        // `&mut self` access for `rand_interval`.
        let due_at = if let Some(t) = self.next_fire_at {
            t
        } else {
            let dwell = self.rand_interval(SOLILOQUY_INTERVAL_MIN_MS, SOLILOQUY_INTERVAL_MAX_MS);
            let t = now + dwell;
            self.next_fire_at = Some(t);
            t
        };

        if now < due_at {
            return;
        }

        // Yield to a non-expired bubble already on screen. The
        // soliloquy beat is the lowest-priority bubble producer:
        // anything that drove the bubble field this tick (or earlier
        // ticks within its TTL) is by definition more topical than a
        // random ambient line. Re-roll the schedule as if we'd fired
        // so the operator-driven bubble's TTL gets to play out before
        // we try again.
        if entity.face.bubble.is_some_and(|b| !b.is_expired(now)) {
            let next = self.rand_interval(SOLILOQUY_INTERVAL_MIN_MS, SOLILOQUY_INTERVAL_MAX_MS);
            self.next_fire_at = Some(now + next);
            return;
        }

        // Pick a phrase. Modulo over a small slice is fine — the
        // tiny bias toward early entries is invisible at 10
        // entries.
        let idx = (self.next_u32() as usize) % SOLILOQUY_LINES.len();
        let text = SOLILOQUY_LINES[idx];
        entity.face.bubble = Some(BubbleState::hold_for(text, now, SOLILOQUY_BUBBLE_TTL_MS));

        // Schedule the next fire from `now`, not from the stale due
        // time, so a long stall (e.g. flash erase) doesn't queue a
        // burst of catch-up fires.
        let next = self.rand_interval(SOLILOQUY_INTERVAL_MIN_MS, SOLILOQUY_INTERVAL_MAX_MS);
        self.next_fire_at = Some(now + next);
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::panic,
    clippy::unwrap_used,
    reason = "test-only: fixture-derived non-zero seeds and bubble unwraps"
)]
mod tests {
    use super::*;

    fn at(now_ms: u64) -> Entity {
        let mut e = Entity::default();
        e.tick.now = Instant::from_millis(now_ms);
        e
    }

    #[test]
    fn disabled_never_writes_bubble() {
        let mut m = Soliloquy::new();
        let mut entity = at(0);
        for t_ms in (0..200_000).step_by(1_000) {
            entity.tick.now = Instant::from_millis(t_ms);
            m.update(&mut entity);
            assert!(
                entity.face.bubble.is_none(),
                "disabled soliloquy must never set bubble; saw it at {t_ms}ms",
            );
        }
    }

    #[test]
    fn enabled_holds_until_first_interval() {
        let mut m = Soliloquy::with_enabled(true);
        let mut entity = at(0);
        // Sample for a window strictly less than the minimum interval.
        for t_ms in (0..SOLILOQUY_INTERVAL_MIN_MS - 100).step_by(500) {
            entity.tick.now = Instant::from_millis(t_ms);
            m.update(&mut entity);
            assert!(
                entity.face.bubble.is_none(),
                "soliloquy fired before SOLILOQUY_INTERVAL_MIN_MS at {t_ms}ms",
            );
        }
    }

    #[test]
    fn enabled_fires_within_max_interval() {
        // Within the upper bound + one bubble TTL we must observe at
        // least one bubble write.
        let mut m = Soliloquy::with_enabled(true);
        let mut entity = at(0);
        let mut saw_bubble = false;
        for t_ms in (0..SOLILOQUY_INTERVAL_MAX_MS + SOLILOQUY_BUBBLE_TTL_MS).step_by(1_000) {
            entity.tick.now = Instant::from_millis(t_ms);
            m.update(&mut entity);
            if entity.face.bubble.is_some() {
                saw_bubble = true;
                break;
            }
        }
        assert!(
            saw_bubble,
            "no soliloquy fired within the max-interval window"
        );
    }

    #[test]
    fn fired_bubble_carries_a_known_phrase_with_correct_ttl() {
        // Drive past the first fire deadline, then verify the bubble
        // text is a member of SOLILOQUY_LINES and the expiry matches
        // SOLILOQUY_BUBBLE_TTL_MS from the fire instant.
        let mut m = Soliloquy::with_enabled(true);
        let mut entity = at(0);
        let mut fire_at: Option<u64> = None;
        for t_ms in (0..SOLILOQUY_INTERVAL_MAX_MS + 100).step_by(500) {
            entity.tick.now = Instant::from_millis(t_ms);
            m.update(&mut entity);
            if entity.face.bubble.is_some() {
                fire_at = Some(t_ms);
                break;
            }
        }
        let fire_ms = fire_at.expect("soliloquy must fire within max-interval window");
        let bubble = entity.face.bubble.expect("bubble set on fire");
        assert!(
            SOLILOQUY_LINES.contains(&bubble.text),
            "bubble text {:?} not in SOLILOQUY_LINES",
            bubble.text,
        );
        assert_eq!(
            bubble.expires_at,
            Instant::from_millis(fire_ms + SOLILOQUY_BUBBLE_TTL_MS),
        );
    }

    #[test]
    fn distinct_seeds_produce_distinct_first_fire_instants() {
        let mut a = Soliloquy::with_seed(true, NonZeroU32::new(0x1234_5678).expect("non-zero"));
        let mut b = Soliloquy::with_seed(true, NonZeroU32::new(0xCAFE_BABE).expect("non-zero"));

        // Drive both, record the first `t_ms` that produced a bubble.
        let first_fire = |m: &mut Soliloquy| -> u64 {
            let mut e = at(0);
            for t_ms in (0..SOLILOQUY_INTERVAL_MAX_MS + 100).step_by(500) {
                e.tick.now = Instant::from_millis(t_ms);
                m.update(&mut e);
                if e.face.bubble.is_some() {
                    return t_ms;
                }
            }
            panic!("soliloquy never fired");
        };
        let fa = first_fire(&mut a);
        let fb = first_fire(&mut b);
        assert_ne!(
            fa, fb,
            "two distinct seeds produced identical first-fire instants",
        );
    }

    #[test]
    fn yields_to_non_expired_bubble_from_other_source() {
        // An MCP `speak` or operator-set bubble should NOT be
        // clobbered by a soliloquy fire. Drive the modifier past its
        // first scheduled fire while a non-soliloquy bubble is
        // active; the modifier must leave the existing bubble alone.
        let mut m = Soliloquy::with_enabled(true);
        let mut entity = at(0);

        // Plant a non-soliloquy bubble whose TTL extends past the
        // soliloquy max-interval so we can be sure the active-bubble
        // gate is what's preventing the fire.
        let external_text = "external operator text";
        entity.face.bubble = Some(BubbleState::hold_for(
            external_text,
            Instant::from_millis(0),
            SOLILOQUY_INTERVAL_MAX_MS + 30_000,
        ));

        for t_ms in (0..SOLILOQUY_INTERVAL_MAX_MS + 1_000).step_by(500) {
            entity.tick.now = Instant::from_millis(t_ms);
            m.update(&mut entity);
            let bubble = entity.face.bubble.expect("external bubble must persist");
            assert_eq!(
                bubble.text, external_text,
                "soliloquy clobbered an active external bubble at {t_ms}ms",
            );
        }
    }

    #[test]
    fn default_matches_new_disabled() {
        let from_default = <Soliloquy as Default>::default();
        let from_new = Soliloquy::new();
        assert_eq!(from_default.enabled, from_new.enabled);
        assert!(
            !from_default.enabled,
            "Soliloquy::new() defaults to disabled"
        );
    }

    #[test]
    fn meta_declares_decoration_phase_and_bubble_write() {
        let m = Soliloquy::new();
        let meta = m.meta();
        assert_eq!(meta.name, "Soliloquy");
        assert_eq!(meta.phase, Phase::Decoration);
        assert_eq!(meta.priority, 0);
        assert_eq!(meta.writes, &[Field::Bubble]);
        assert_eq!(meta.reads, &[Field::Bubble]);
    }

    #[test]
    fn rand_interval_returns_lo_when_hi_not_greater() {
        // Degenerate bounds: `hi <= lo` must short-circuit and return
        // `lo` rather than panic on the unsigned subtraction in the
        // `span` calculation.
        let mut m = Soliloquy::with_seed(true, NonZeroU32::new(1).expect("non-zero"));
        assert_eq!(m.rand_interval(7, 7), 7);
        assert_eq!(m.rand_interval(100, 50), 100);
    }
}
