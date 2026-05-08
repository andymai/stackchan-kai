# Dance choreography

Stackchan-kai accepts a JSON keyframe stream over `POST /dance` and
sequences head pose, emotion, decorator, and LED-ring colour through
the `DancePlayer` modifier.

## Wire format

```text
POST /dance
Content-Type: application/json

{
  "keyframes": [
    {"at_ms": 0,    "emotion": "happy",        "r": 255, "g": 200, "b":   0},
    {"at_ms": 0,    "pan_deg": -20.0, "tilt_deg": 10.0},
    {"at_ms": 500,  "pan_deg":  20.0},
    {"at_ms": 1000, "pan_deg": -20.0, "decorator": "heart"},
    {"at_ms": 1500, "pan_deg":   0.0, "tilt_deg": 0.0, "r": 0, "g": 0, "b": 255}
  ]
}
```

Schema (host-defined in
[`stackchan_core::dance`](../crates/stackchan-core/src/dance.rs);
parser in
[`stackchan_net::dance`](../crates/stackchan-net/src/dance.rs)):

| Field       | Type      | Required | Notes |
|-------------|-----------|----------|-------|
| `at_ms`     | `u32`     | yes      | Offset from script start, in milliseconds. Keyframes must be in `at_ms` order; equal values are allowed (different channels firing simultaneously). |
| `pan_deg`   | `f32`     | no       | Head pan target. Clamped at the head driver to `±MAX_PAN_DEG`. |
| `tilt_deg`  | `f32`     | no       | Head tilt target. Clamped to `[MIN_TILT_DEG, MAX_TILT_DEG]`. |
| `emotion`   | `string`  | no       | Lowercase variant — same vocabulary as `POST /emotion`. |
| `decorator` | `string`  | no       | Lowercase variant — `heart` / `sweat` / `dizzy` / `ear` / `pairing` / `angry` / `shy`. |
| `r`,`g`,`b` | `u8`      | no       | LED-ring colour. Set all three or none — partial triples are rejected. |

A response is `204 No Content` on a parsed-and-loaded script, or
`400 Bad Request` with the parser error in the body otherwise.

## Sampling semantics

- Channels sample independently. A keyframe that only sets `pan_deg`
  leaves the avatar and RGB channels alone, and the player keeps
  using whatever the most-recent matching keyframe supplied for those
  channels.
- The player picks the most-recent keyframe at-or-before the current
  elapsed offset per channel — no interpolation between keyframes.
  The `SCServo`'s 20 ms move-time interpolator smooths motion-channel
  step changes implicitly.
- Avatar emotion writes the autonomy gate so `EmotionCycle` doesn't
  advance mid-dance. The gate is refreshed each tick the player
  asserts an emotion, so any pause in emotion-keyframe coverage
  releases autonomy gracefully.
- Head pose is layered additively on top of upstream Motion modifiers
  (idle drift, emotion bias, attention follow). Pin emotion to
  `Neutral` in the script's first keyframe to zero out the
  emotion-driven bias for the dance window.

## Lifecycle

1. Operator `POST`s the script. The HTTP handler signals it to the
   render task, which writes it to `entity.input.dance_script`.
2. The `DancePlayer` modifier consumes the input slot on the next
   tick and anchors at the current instant.
3. Each tick the player samples and writes overrides.
4. After the last keyframe + a 200 ms tail the player releases all
   overrides and the rest of the modifier stack resumes.

A new upload mid-dance replaces the active script — the previous
script's overrides are released on the same tick the new one anchors.

## Limits

- `MAX_KEYFRAMES = 1024` per script (~30 seconds at 30 ms cadence).
- `Arc<DanceScript>` handoff is `O(1)` regardless of keyframe count.
- The author can trim keyframes to a subset of channels — the player
  treats unset fields as "carry the prior value forward" rather than
  "clear", so a 200 ms emotion-only beat doesn't disturb a 50 ms
  motion sweep running underneath.
