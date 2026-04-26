---
title: Naming conventions
---

# Naming conventions

Five rules govern names across `stackchan-core` (and the firmware /
sim that consume it). Each is grounded in prior art from a similar
codebase — citations inline so future contributors can re-read the
source if a rule needs to bend.

## Rule A — `Intent` variants

`Intent` describes the avatar's **current state**. Use:

- **Past participle** when an external action caused the state.
  `PickedUp`, `Shaken`, `Tilted`, `Petted`, `Startled`.
- **Gerund** when it's the avatar's own ongoing activity.
  `Listening`.
- **Bare adjective** for the resting default. `Idle`.

The split mirrors [Embassy's `UsbDeviceState`][embassy-usb-state]
(`Configured`, `Addressed`, `Disabled` — externally caused →
past-participle) and the
[Unity / Unreal / Spine animation-state-machine][unity-state-machines]
convention (`Idle` / `Walking` / `Running` — own activity → gerund).

Avoid bare verbs (`Listen`, `Wake`) and noun-adjective compounds
(`HearingLoud`) — they read as an action being requested rather than
a state being held.

[embassy-usb-state]: https://docs.embassy.dev/embassy-usb/git/default/enum.UsbDeviceState.html
[unity-state-machines]: https://docs.unity3d.com/2021.2/Documentation/Manual/StateMachineBasics.html

## Rule B — Modifier names

Two grammatical classes by what the modifier *does to the avatar*:

- **Bare noun** for autonomous behaviors that read time + entity
  state and produce output without an external trigger:
  `Blink`, `Breath`, `IdleHeadDrift`, `IdleDrift`, `EmotionCycle`.
  Mirrors [Bevy components][bevy-builtins] (no `*Component` suffix).

- **`<Output>From<Source>`** for translators that map an input
  field/signal to an output field. Reads like a function signature
  — output is the return value, source is the argument:
  `EmotionFromTouch`, `HeadFromEmotion`, `StyleFromIntent`,
  `IntentFromLoud`, `MouthFromAudio`.

The `<Output>` half names the *field* being written, not the
specific value. `EmotionFromVoice` writes `mind.affect.emotion` —
the doc comment specifies the value(s) it picks (`Happy` on
sustained voice). Same for `HeadFromIntent`: the doc comment
specifies which intent variants it reacts to (`Startled`).

No `*Modifier` suffix — the trait registry is typed, so the suffix
is redundant. (The Bevy team explicitly dropped the matching
`*System` suffix; see [`bevy#2804`][bevy-system-naming].)

[bevy-builtins]: https://bevy-cheatbook.github.io/builtins.html
[bevy-system-naming]: https://github.com/bevyengine/bevy/discussions/2804

## Rule C — Skill names

Skills are *long-running detection routines*. Use:

- **Gerund** for recognizers that watch a percept and emit an
  intent / attention. `Listening`, `Petting`, `Handling`.

- **Verb-object** for skills that *act* on a percept (none yet, but
  e.g. `FollowFace`, `TrackMotion` for future skills that drive head
  pose toward a tracked target).

The split mirrors [BehaviorTree.CPP][bt-concepts]'s grammatical
split between conditions (predicates / state) and actions
(imperative verbs), and matches [Cozmo's behavior naming][cozmo-behaviors]
(`FindFaces`, `StackBlocks`).

No `*Skill` / `*Detector` / `*Recognizer` suffix.

[bt-concepts]: https://www.behaviortree.dev/docs/learn-the-basics/main_concepts/
[cozmo-behaviors]: https://github.com/anki/cozmo-python-sdk/blob/master/src/cozmo/behavior.py

## Rule D — `ChirpKind` and event variants

Bare noun for the event itself: `Pickup`, `Wake`, `Startle`. Past
participle for transitions: `CameraModeEntered`, `CameraModeExited`.
The `*Alert` suffix is acceptable when the chirp announces a
*condition*, not an event: `LowBatteryAlert`.

Matches [`serde_json::Value`][serde-value] (bare nouns) and the
animation-state-machine `OnEntered` / `OnExited` convention.

Avoid `*Event` suffix — the enum name (`ChirpKind`, `EmotionFromIntent`,
etc.) carries the kind information.

[serde-value]: https://docs.rs/serde_json/latest/serde_json/enum.Value.html

## Rule E — Cause / state pairing (`OverrideSource` ↔ `Intent`)

When a cause enum and a state enum describe the same root concept,
use the **noun form for the cause** and the **past-participle form
for the state**:

| Cause (`OverrideSource`) | Resulting state (`Intent`)     |
|--------------------------|--------------------------------|
| `Pickup`                 | `PickedUp`                     |
| `Shake`                  | `Shaken`                       |
| `Voice`                  | `Listening`                    |
| `Startle`                | `Startled`                     |
| `BodyTouch`              | `Petted`                       |
| `FaceTouch`              | *(no intent — hold only)*      |
| `Remote`                 | *(no intent — hold only)*      |
| `Ambient`                | *(no intent — emotion only)*   |
| `LowBattery`             | *(no intent — emotion only)*   |

The cause noun and the state past-participle are different
grammatical forms of the same concept, so the pair reads naturally
in context (`source = Pickup, intent = PickedUp`) without colliding
on identifier.

## Current inventory

All names on `main` conform to the rules above. The inventory below
is a quick cross-reference; the canonical lists live in the source
(`crates/stackchan-core/src/{mind,voice,emotion}.rs` and
`crates/stackchan-core/src/{modifiers,skills}/mod.rs`).

- **Intent** (rule A): `Idle`, `Listening`, `Startled`, `Petted`,
  `PickedUp`, `Shaken`, `Tilted`.
- **Emotion** (single-word adjectives, matches the
  [m5stack-avatar `Expression`][m5stack-expression] enum this project
  descends from): `Neutral`, `Happy`, `Sad`, `Sleepy`, `Surprised`,
  `Angry`.
- **Skills** (rule C, all gerund recognizers today): `Listening`,
  `Petting`, `Handling`.
- **ChirpKind** (rule D): `Pickup`, `Wake`, `Startle`,
  `LowBatteryAlert`, `CameraModeEntered`, `CameraModeExited`.
- **OverrideSource** (rule E): `FaceTouch`, `BodyTouch`, `Remote`,
  `Pickup`, `Shake`, `Voice`, `Startle`, `Ambient`, `LowBattery`.
- **Modifiers — autonomous** (rule B, bare noun): `Blink`, `Breath`,
  `IdleHeadDrift`, `IdleDrift`, `EmotionCycle`.
- **Modifiers — translators** (rule B, `<Output>From<Source>`):
  `EmotionFromTouch`, `EmotionFromRemote`, `EmotionFromIntent`,
  `EmotionFromVoice`, `EmotionFromAmbient`, `EmotionFromBattery`,
  `IntentFromBodyTouch`, `IntentFromLoud`, `StyleFromEmotion`,
  `StyleFromIntent`, `HeadFromEmotion`, `HeadFromAttention`,
  `HeadFromIntent`, `MouthFromAudio`.

[m5stack-expression]: https://github.com/stack-chan/m5stack-avatar/blob/master/src/Expression.h

## Forward guidance

### Empty Director phases (`Perception`, `Cognition`, `Speech`, `Output`)

When modifiers land in these phases, apply Rule B. Likely shapes:

- `Phase::Perception` — modifiers that pre-process raw sensor data
  before downstream phases see it. Naming: `<DerivedField>From<RawField>`.
  E.g., a noise-floor gate on `audio_rms` would be
  `AudioGatedFromNoise`, not `NoiseGate`.
- `Phase::Cognition` — modifiers that synthesize across percepts
  into higher-level state. Naming: `<HigherField>From<PerceptCombo>`.
- `Phase::Speech` — modifiers that translate intent / emotion into
  voice queue entries. Naming: `<VoiceField>From<Source>`.
- `Phase::Output` — modifiers that translate logical output into
  hardware-shape commands. Naming: `<HardwareField>From<LogicalField>`.

### New Skills

Apply Rule C: gerund for recognizers, verb-object for actors.
Examples for likely future skills:

- A motion-tracking skill that reads camera frames and writes an
  attention target → `Tracking` (recognizer-style; emits attention).
- A skill that drives head pose to follow a tracked face →
  `FollowFace` (actor-style; takes action).

### New Intents

Apply Rule A:

- States caused by external action → past participle (`Petted`,
  `Tracked`, `Greeted`).
- Own ongoing activity → gerund (`Listening`, `Watching`).
- Resting / default → bare adjective (`Idle`).

### New cause / state pairs

Apply Rule E. If a new sensor produces both a cause-side override
hold and an intent-side state, name them as the noun / past-participle
pair (e.g., `OverrideSource::Tracking` ↔ `Intent::Tracked`).

## Why no `*Modifier` / `*Skill` suffix

The trait registry is typed (`Director::add_modifier(&mut dyn
Modifier)`), so the type system already carries the role. The Bevy
ecosystem proved the suffix is redundant after they dropped `*System`
([discussion][bevy-system-naming]). Unreal keeps `*Component` only
because its registry uses untyped `UObject` reflection — not our
shape.

## When to break a rule

These rules optimize for reader clarity given the current set of
modifiers / skills / intents. If a future name would obey the rule
but read worse than a deliberate exception (e.g., a modifier whose
behavior fundamentally isn't a translator and isn't autonomous —
something genuinely third-shape), make the exception, document the
why in the module-level doc comment, and update this file with the
new pattern if it recurs.

The convention exists to be useful, not to be bureaucratic.
