---
crate: tracker
role: Block-grid motion tracker (RGB565 → head Pose)
bus: none (pure algorithm)
transport: in-memory frame slices
no_std: true
unsafe: forbidden
status: experimental (v0.x)
---

# tracker

`no_std` block-grid motion tracker for the Stack-chan camera. Consumes
raw RGB565 QVGA frames, computes inter-frame motion via per-block luma
deltas, and emits a target `stackchan_core::Pose` for the head servos.
Pure algorithm — no I/O, no allocation, host-testable from canned
fixture frames.

## Key Files

- `src/lib.rs` — `TrackerConfig`, `Tracker`, `Outcome`, `Motion`,
  control-law (P-gain + dead zone + per-step slew + idle-timeout
  return-to-centre), inline unit tests on synthesised frames
- `src/luma.rs` — RGB565 → 8-bit luma approximation
  (`(R8 + 2·G8 + B8) >> 2`), `fill_block_luma` reduction over a
  configurable `blocks_x` × `blocks_y` grid (≤ 16 × 16)

## Algorithm

For each step:

1. **Per-block mean luma.** Reduce the frame into a small grid (default
   8 × 6 → 40 × 40 pixel cells over QVGA). Luma uses a fast
   shifts-only Rec. 601 approximation.
2. **Per-block delta vs. previous frame.** A block whose normalised
   delta exceeds `block_threshold` "fires".
3. **Centroid of fired cells.** Mapped to `[-1, 1]` per axis.
4. **Reject global events.** If too many cells fire (default > 70%),
   the frame is treated as a lighting flip and the pose held.
5. **Dead zone + P-gain + slew clamp.** Centroid is converted to a pan
   / tilt delta via the configured camera FOV; small offsets pass
   through the dead zone untouched, the rest are scaled by `p_gain`
   and clamped to `±max_step_deg`.
6. **Idle timeout.** After `idle_timeout_ms` of no motion the target
   pose slews back toward `Pose::NEUTRAL` at `idle_step_deg` per step.
7. **`Pose::clamped`.** Final assignment routes through the
   stackchan-core safe-range clamp (asymmetric tilt — see
   `stackchan_core::head`).

## Sign Conventions

Inherited from `stackchan_core::head`:

- `+pan_deg` → head turns *right* from the viewer's POV.
- `+tilt_deg` → head nods *up* (chin rises). `MIN_TILT_DEG = 0`, so
  the tracker can ask for a downward look but `Pose::clamped` pins
  it to level.
- Centroid `nx > 0` ⇒ motion right of frame centre ⇒ pan delta `> 0`.
- Centroid `ny > 0` ⇒ motion below frame centre ⇒ tilt delta `< 0`
  (head nods down) — clamped to 0 in practice.

## Coupling

- Depends on `stackchan-core` *only* for `Pose` and the safe-range
  constants. No firmware deps; runs unchanged on host.
- The firmware's `examples/tracker_bench.rs` exercises this crate
  end-to-end against the live GC0308 + LCD\_CAM camera task and logs
  proposed poses without driving any servo.

## Tuning

Defaults live in `TrackerConfig::DEFAULT` and are tuned for QVGA
GC0308 + Stack-chan SCServo head on a CoreS3. Edit per axis as
needed; the bench logs centroid + fired-cell counts every step so
empirical tuning is straightforward.

## Limitations

- Frame-difference detection localises to the **midpoint** of motion,
  not the new position — fast left-to-right travel of an object will
  fire blocks at *both* ends, biasing the centroid toward the centre.
  For Stack-chan use (people entering frame, slow scene-rate change)
  this works well; longer-term, a running-mean background subtraction
  would track moving objects more accurately.
- 8 × 6 grid resolution puts angular resolution at ~7° per cell on a
  62° H FOV — fine enough for slow head tracking, coarse for precise
  saccades.
