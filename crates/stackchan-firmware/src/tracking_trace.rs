//! Camera-tracking pipeline observability.
//!
//! Compiled to a zero-size no-op when the `tracking-trace` feature
//! is off, so the production firmware build pays no runtime cost.
//! Enable with `cargo +esp build --release --features tracking-trace`
//! (or `just fmr-trace`) when measuring the lock/release path on a
//! live unit.
//!
//! ## What it traces
//!
//! After each `Director::run` in the render task, [`TraceState::observe`]
//! compares this tick's `entity.mind.attention` and `entity.mind.engagement`
//! against the previous tick's snapshot and emits a structured `defmt`
//! event on every transition. It also tracks lock-fire latency (time
//! from the first `Tracking` observation in a burst to the
//! `Attention::Tracking` lock) and a periodic observation cadence
//! gauge.
//!
//! All emitted events use the prefix `trk:` so they're easy to filter
//! with `espflash monitor … | grep trk:` (or via a defmt log filter
//! once that lands in espflash).
//!
//! ## Why firmware-side, not core?
//!
//! Keeping the trace logic out of `stackchan-core` preserves core's
//! `no_std` + defmt-free posture and avoids dragging the dep into the
//! sim and host-test paths. The firmware already runs the Director;
//! it just needs to look at the entity afterward.

use stackchan_core::Entity;

#[cfg(feature = "tracking-trace")]
use stackchan_core::{Attention, Engagement, Instant};

/// Per-render-tick state for transition detection.
///
/// When the `tracking-trace` feature is **off**, this is a zero-size
/// type and all methods compile to no-ops. When **on**, it carries
/// the prior tick's attention + engagement snapshots and the
/// observation-counter / cadence-gauge state.
///
/// Construct once in the render task; call [`Self::note_observation`]
/// whenever the firmware drains a `TrackingObservation` from the
/// camera signal, and [`Self::observe`] right after `Director::run`.
#[cfg(feature = "tracking-trace")]
#[derive(Debug)]
pub struct TraceState {
    /// Previous-tick attention snapshot. Used for edge detection.
    prev_attention: Attention,
    /// Previous-tick engagement snapshot. Used for edge detection
    /// against the variant kind (we don't compare the inner Pose /
    /// centroid because both change every tick during a lock).
    prev_engagement: EngagementKind,
    /// `Some(t)` once the firmware has drained a Tracking-class
    /// observation since the last attention release; `None` between
    /// bursts. Used to measure lock-fire latency.
    burst_started_at: Option<Instant>,
    /// Observations seen since the last cadence report. Reset on
    /// each periodic emission.
    obs_count: u32,
    /// Wall-clock millis at the last cadence report. The first call
    /// to [`Self::observe`] anchors this so the first interval is
    /// measured from boot.
    last_cadence_at_ms: Option<u64>,
}

#[cfg(feature = "tracking-trace")]
impl TraceState {
    /// Cadence gauge interval, in ms. Two seconds is short enough to
    /// notice a stalled camera task quickly, long enough to avoid
    /// flooding the JTAG channel.
    pub const CADENCE_REPORT_INTERVAL_MS: u64 = 2_000;

    /// Construct a fresh trace state. Both prev-state slots seed at
    /// the boot defaults (`Attention::None`, `Engagement::Idle`), so
    /// the first transition into either non-default state fires a
    /// trace.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            prev_attention: Attention::None,
            prev_engagement: EngagementKind::Idle,
            burst_started_at: None,
            obs_count: 0,
            last_cadence_at_ms: None,
        }
    }

    /// Note that the firmware just drained a `TrackingObservation`.
    /// Drives the cadence gauge and seeds the lock-fire latency
    /// stopwatch on the first observation of a burst.
    ///
    /// The anchor seeds whenever no burst is in flight AND attention
    /// is *not* already `Tracking`. Anchoring only on `Attention::None`
    /// would silently miss the common "audio engagement, then face
    /// arrives" handoff (`Listening` → `Tracking`) — those bursts'
    /// `lock_latency_ms` would never emit. Anchoring while `Tracking`
    /// would reset the stopwatch mid-burst on every fresh observation.
    pub const fn note_observation(&mut self, now: Instant) {
        self.obs_count = self.obs_count.saturating_add(1);
        if self.burst_started_at.is_none()
            && !matches!(self.prev_attention, Attention::Tracking { .. })
        {
            self.burst_started_at = Some(now);
        }
    }

    /// Observe entity state after `Director::run` and emit any
    /// transition or cadence events.
    pub fn observe(&mut self, entity: &Entity, now: Instant) {
        let now_ms = now.as_millis();

        let curr_attention = entity.mind.attention;
        let curr_engagement = EngagementKind::from(&entity.mind.engagement);
        let prev_kind = AttentionKind::from(&self.prev_attention);
        let curr_kind = AttentionKind::from(&curr_attention);

        if prev_kind != curr_kind {
            defmt::info!(
                "trk: attention {} -> {} @ {}ms",
                prev_kind,
                curr_kind,
                now_ms
            );
            // Lock-fire latency: report the (start_of_burst -> lock) interval
            // the first time attention enters Tracking.
            if matches!(curr_attention, Attention::Tracking { .. })
                && let Some(start) = self.burst_started_at
            {
                let latency = now_ms.saturating_sub(start.as_millis());
                defmt::info!("trk: lock_latency_ms={}", latency);
            }
            // Reset on either Tracking (we just emitted latency) or
            // None (release) so the next burst gets a fresh anchor.
            if matches!(curr_attention, Attention::Tracking { .. } | Attention::None) {
                self.burst_started_at = None;
            }
        }

        if self.prev_engagement != curr_engagement {
            defmt::info!(
                "trk: engagement {} -> {} @ {}ms",
                self.prev_engagement,
                curr_engagement,
                now_ms
            );
        }

        let last = *self.last_cadence_at_ms.get_or_insert(now_ms);
        if now_ms.saturating_sub(last) >= Self::CADENCE_REPORT_INTERVAL_MS {
            let elapsed_ms = (now_ms - last).max(1);
            let per_sec = (u64::from(self.obs_count).saturating_mul(1_000)) / elapsed_ms;
            defmt::debug!(
                "trk: obs={} over {}ms (~{}/s)",
                self.obs_count,
                elapsed_ms,
                per_sec
            );
            self.obs_count = 0;
            self.last_cadence_at_ms = Some(now_ms);
        }

        self.prev_attention = curr_attention;
        self.prev_engagement = curr_engagement;
    }
}

#[cfg(feature = "tracking-trace")]
impl Default for TraceState {
    fn default() -> Self {
        Self::new()
    }
}

/// `defmt::Format`-friendly projection of [`Attention`]. Bare variant
/// names without payload — Pose + Instant inside Tracking aren't
/// useful for transition logs and would bloat each frame. Future
/// `Attention` variants (the enum is `#[non_exhaustive]`) fall back
/// to `Unknown` so they'd be visible in the log rather than silently
/// matching an existing kind.
#[cfg(feature = "tracking-trace")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, defmt::Format)]
enum AttentionKind {
    /// No focus.
    None,
    /// Listening to a sound source.
    Listening,
    /// Tracking a moving target.
    Tracking,
    /// Attention variant added in core after this build was compiled.
    Unknown,
}

#[cfg(feature = "tracking-trace")]
impl From<&Attention> for AttentionKind {
    fn from(a: &Attention) -> Self {
        match a {
            Attention::None => Self::None,
            Attention::Listening { .. } => Self::Listening,
            Attention::Tracking { .. } => Self::Tracking,
            _ => Self::Unknown,
        }
    }
}

/// `defmt::Format`-friendly projection of [`Engagement`]. Bare
/// variant names without payload — centroid + Instant change every
/// tick and aren't useful in transition logs. Future variants fall
/// back to `Unknown` (see [`AttentionKind`]).
#[cfg(feature = "tracking-trace")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, defmt::Format)]
enum EngagementKind {
    /// No face seen recently.
    Idle,
    /// Face seen for fewer than the lock-hits threshold.
    Locking,
    /// Face lock engaged.
    Locked,
    /// Face missed but inside the release-misses window.
    Releasing,
    /// Engagement variant added in core after this build was compiled.
    Unknown,
}

#[cfg(feature = "tracking-trace")]
impl From<&Engagement> for EngagementKind {
    fn from(e: &Engagement) -> Self {
        match e {
            Engagement::Idle => Self::Idle,
            Engagement::Locking { .. } => Self::Locking,
            Engagement::Locked { .. } => Self::Locked,
            Engagement::Releasing { .. } => Self::Releasing,
            _ => Self::Unknown,
        }
    }
}

// ---------- No-op variant for production builds ----------

/// Zero-size no-op variant. All methods compile to nothing when the
/// `tracking-trace` feature is off, so production builds carry no
/// runtime cost.
#[cfg(not(feature = "tracking-trace"))]
#[derive(Debug, Default)]
pub struct TraceState;

#[cfg(not(feature = "tracking-trace"))]
impl TraceState {
    /// Construct the no-op trace state.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// No-op stub matching the feature-on signature.
    #[allow(
        clippy::unused_self,
        clippy::needless_pass_by_value,
        reason = "matches the feature-on signature so callers compile under both configs"
    )]
    pub const fn note_observation(&mut self, _now: stackchan_core::Instant) {}

    /// No-op stub matching the feature-on signature.
    #[allow(
        clippy::unused_self,
        reason = "matches the feature-on signature so callers compile under both configs"
    )]
    pub const fn observe(&mut self, _entity: &Entity, _now: stackchan_core::Instant) {}
}
