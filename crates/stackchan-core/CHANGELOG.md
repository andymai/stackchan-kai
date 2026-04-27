# Changelog

## [0.16.0](https://github.com/andymai/stackchan-kai/compare/stackchan-core-v0.15.0...stackchan-core-v0.16.0) (2026-04-27)


### Features

* **firmware:** POST /speak + audio queue eviction policy ([#163](https://github.com/andymai/stackchan-kai/issues/163)) ([9024675](https://github.com/andymai/stackchan-kai/commit/902467580c17c69190c6c148231bc057ce677c73))

## [0.15.0](https://github.com/andymai/stackchan-kai/compare/stackchan-core-v0.14.0...stackchan-core-v0.15.0) (2026-04-27)


### Features

* **firmware:** HTTP POST control plane (emotion, look-at, reset) ([#152](https://github.com/andymai/stackchan-kai/issues/152)) ([f2f9076](https://github.com/andymai/stackchan-kai/commit/f2f9076edb846f90381b75fbb991632f7331d8a0))

## [0.14.0](https://github.com/andymai/stackchan-kai/compare/stackchan-core-v0.13.0...stackchan-core-v0.14.0) (2026-04-26)


### Features

* speech synthesis framework with baked backend ([#132](https://github.com/andymai/stackchan-kai/issues/132)) ([8c9018f](https://github.com/andymai/stackchan-kai/commit/8c9018f709f80ea4e24d0215ca8e57d7c3a5dd8c))

## [0.13.0](https://github.com/andymai/stackchan-kai/compare/stackchan-core-v0.12.0...stackchan-core-v0.13.0) (2026-04-26)


### Features

* **core:** dormant mode quiets head servos when nothing's happening ([#128](https://github.com/andymai/stackchan-kai/issues/128)) ([1d60ae9](https://github.com/andymai/stackchan-kai/commit/1d60ae902ed1cdb0d26129ede92105912e0aa70d))

## [0.12.0](https://github.com/andymai/stackchan-kai/compare/stackchan-core-v0.11.0...stackchan-core-v0.12.0) (2026-04-26)


### Features

* face tracking with engagement-driven gaze and lost-target search ([#121](https://github.com/andymai/stackchan-kai/issues/121)) ([142fc1c](https://github.com/andymai/stackchan-kai/commit/142fc1c9e2bdc161a2d5c36492f0fec85a3dcbfe))

## [0.11.0](https://github.com/andymai/stackchan-kai/compare/v0.10.0...v0.11.0) (2026-04-26)


### Features

* **core:** tracking realism — multi-target detection + microsaccades + eye-leads-head + engagement-aware blink/breath ([#115](https://github.com/andymai/stackchan-kai/issues/115)) ([e667e26](https://github.com/andymai/stackchan-kai/commit/e667e263c351791eb35686910b984485ade47871))


### Bug Fixes

* **core:** smooth head tracking with single-pole low-pass ([#116](https://github.com/andymai/stackchan-kai/issues/116)) ([3d6e2b0](https://github.com/andymai/stackchan-kai/commit/3d6e2b0b5094d14cd6e703edfef6701689504671))
* **core:** treat Holding as lock-eligible so brief motion engages tracking ([#117](https://github.com/andymai/stackchan-kai/issues/117)) ([75e86c2](https://github.com/andymai/stackchan-kai/commit/75e86c246439817f1ae8c818193dc46845e3ea0f))

## [0.10.0](https://github.com/andymai/stackchan-kai/compare/v0.9.7...v0.10.0) (2026-04-26)


### ⚠ BREAKING CHANGES

* engine architecture — Entity + Director + Modifier/Skill ([#90](https://github.com/andymai/stackchan-kai/issues/90))


### Features

* **core:** LookAtSound skill + ListenHead motion modifier ([#92](https://github.com/andymai/stackchan-kai/issues/92))
* **core:** debug-mode enforcement of ModifierMeta + SkillMeta writes ([#95](https://github.com/andymai/stackchan-kai/issues/95))
* **core:** BodyGesture modifier — Press/Swipe/Release on Si12T strip ([#101](https://github.com/andymai/stackchan-kai/issues/101))
* **core:** Petting skill — sustained body-touch → mind.intent=Petted ([#102](https://github.com/andymai/stackchan-kai/issues/102))
* **core:** IntentStyle modifier — visible reaction to mind.intent ([#103](https://github.com/andymai/stackchan-kai/issues/103))
* **core:** Handling skill — IMU → mind.intent (PickedUp/Shaken/Tilted) ([#105](https://github.com/andymai/stackchan-kai/issues/105))
* **core:** StartleOnLoud modifier — sound-reactive startle chain ([#106](https://github.com/andymai/stackchan-kai/issues/106))
* **core:** perception.tracking field + firmware drain ([#110](https://github.com/andymai/stackchan-kai/issues/110))
* **core:** AttentionFromTracking — Cognition modifier + Attention::Tracking ([#111](https://github.com/andymai/stackchan-kai/issues/111))
* **core:** head + eye reactions to Attention::Tracking ([#112](https://github.com/andymai/stackchan-kai/issues/112))


### Bug Fixes

* **core:** repair Style::* intra-doc links in draw.rs (CI hotfix) ([#93](https://github.com/andymai/stackchan-kai/issues/93))


### Refactors

* apply naming convention sweep across the inventory ([#108](https://github.com/andymai/stackchan-kai/issues/108))
* **core:** align ModifierMeta reads/writes with actual access ([#94](https://github.com/andymai/stackchan-kai/issues/94))

## [0.9.0](https://github.com/andymai/stackchan-kai/compare/v0.8.0...v0.9.0) (2026-04-25)


### Features

* BM8563 wall-clock + LTR-553 AmbientSleepy modifier ([#18](https://github.com/andymai/stackchan-kai/issues/18)) ([8405d0d](https://github.com/andymai/stackchan-kai/commit/8405d0d0f1619d400ff2bd1691995135f9c25316))
* BMI270 IMU + pickup-reaction modifier ([#17](https://github.com/andymai/stackchan-kai/issues/17)) ([8624ceb](https://github.com/andymai/stackchan-kai/commit/8624ceb92bcb355a28fa3c98ee6499171cb12a2b))
* BMM150 magnetometer (9-axis data path) ([#22](https://github.com/andymai/stackchan-kai/issues/22)) ([eea9212](https://github.com/andymai/stackchan-kai/commit/eea921233444d2efe68d7ee502e14be390778c20))
* **core:** MouthOpenAudio modifier + Mouth::mouth_open field ([#32](https://github.com/andymai/stackchan-kai/issues/32)) ([79020ed](https://github.com/andymai/stackchan-kai/commit/79020ed266f510b1bd2da1f7ecc01f8465105737))
* **core:** render Avatar onto any embedded-graphics DrawTarget ([a8d9b80](https://github.com/andymai/stackchan-kai/commit/a8d9b80a65ef8557d0cd829eadd873246b434138))
* **core:** wire Emotion into a style-field pipeline with eased transitions ([bfd6a3a](https://github.com/andymai/stackchan-kai/commit/bfd6a3a168ad8f6bcece0e5bfc47f01e791ab8ff))
* **draw:** visibly render Mouth::mouth_open as growing ellipse ([#33](https://github.com/andymai/stackchan-kai/issues/33)) ([c21f9da](https://github.com/andymai/stackchan-kai/commit/c21f9da786472e94dbbe5e307dd052e172ebf646))
* emotion-coupled head motion (EmotionHead modifier) ([#4](https://github.com/andymai/stackchan-kai/issues/4)) ([3f197f1](https://github.com/andymai/stackchan-kai/commit/3f197f106527977da99cdd9ac75dab79462290c4))
* emotion-transition chirps — pickup, wake, low-battery audio cues ([#56](https://github.com/andymai/stackchan-kai/issues/56)) ([f097f8c](https://github.com/andymai/stackchan-kai/commit/f097f8c78f0e8299f088e8aac8180a17b89ad623))
* FT6336U tap-to-cycle emotion + shared I²C0 bus ([#15](https://github.com/andymai/stackchan-kai/issues/15)) ([b3fb8de](https://github.com/andymai/stackchan-kai/commit/b3fb8de289ac45f9c5537516a79c2dd2e3a4e6cb))
* low-battery hysteresis + USB-power aware override ([#54](https://github.com/andymai/stackchan-kai/issues/54)) ([0741a2e](https://github.com/andymai/stackchan-kai/commit/0741a2edcb9478c34e1fdb4e6739393cedfd2019))
* low-battery sleepy emotion — AXP2101 SoC reader, power task, modifier ([#52](https://github.com/andymai/stackchan-kai/issues/52)) ([d6add63](https://github.com/andymai/stackchan-kai/commit/d6add63fa0a16dc17cb5984c163eab80600f0885))
* pan/tilt servo head motion (+aw9523 extract, pca9685 driver) ([#2](https://github.com/andymai/stackchan-kai/issues/2)) ([9bc40a1](https://github.com/andymai/stackchan-kai/commit/9bc40a11b76108aad8a58af7a941a28fade5ea0c))
* power-button taps + IR NEC RemoteCommand modifier ([#19](https://github.com/andymai/stackchan-kai/issues/19)) ([f29c92e](https://github.com/andymai/stackchan-kai/commit/f29c92e7550fa450be0adb8b94a8b57801e18ddf))
* PY32 WS2812 LED ring + first output-sink path ([#20](https://github.com/andymai/stackchan-kai/issues/20)) ([14dcbd3](https://github.com/andymai/stackchan-kai/commit/14dcbd3be257175f0df973c5cb616c41d92c483b))
* servo position readback + calibration bench binary ([#11](https://github.com/andymai/stackchan-kai/issues/11)) ([890c8f8](https://github.com/andymai/stackchan-kai/commit/890c8f8232f80f3b6f861ded1ec2b2e386fbc31d))
* WakeOnVoice modifier — sustained mic activity wakes to Happy ([#55](https://github.com/andymai/stackchan-kai/issues/55)) ([c8729bf](https://github.com/andymai/stackchan-kai/commit/c8729bfbac9d78de54f20c64875d42e8544d0b8c))


### Bug Fixes

* tilt calibration for offset-encoder unit + asymmetric range support ([#47](https://github.com/andymai/stackchan-kai/issues/47)) ([52b8c4d](https://github.com/andymai/stackchan-kai/commit/52b8c4d47477baf776c82446d02431d08d24f941))

## [0.8.0](https://github.com/andymai/stackchan-kai/compare/v0.7.0...v0.8.0) (2026-04-25)


### Features

* BM8563 wall-clock + LTR-553 AmbientSleepy modifier ([#18](https://github.com/andymai/stackchan-kai/issues/18)) ([8405d0d](https://github.com/andymai/stackchan-kai/commit/8405d0d0f1619d400ff2bd1691995135f9c25316))
* BMI270 IMU + pickup-reaction modifier ([#17](https://github.com/andymai/stackchan-kai/issues/17)) ([8624ceb](https://github.com/andymai/stackchan-kai/commit/8624ceb92bcb355a28fa3c98ee6499171cb12a2b))
* BMM150 magnetometer (9-axis data path) ([#22](https://github.com/andymai/stackchan-kai/issues/22)) ([eea9212](https://github.com/andymai/stackchan-kai/commit/eea921233444d2efe68d7ee502e14be390778c20))
* **core:** MouthOpenAudio modifier + Mouth::mouth_open field ([#32](https://github.com/andymai/stackchan-kai/issues/32)) ([79020ed](https://github.com/andymai/stackchan-kai/commit/79020ed266f510b1bd2da1f7ecc01f8465105737))
* **core:** render Avatar onto any embedded-graphics DrawTarget ([a8d9b80](https://github.com/andymai/stackchan-kai/commit/a8d9b80a65ef8557d0cd829eadd873246b434138))
* **core:** wire Emotion into a style-field pipeline with eased transitions ([bfd6a3a](https://github.com/andymai/stackchan-kai/commit/bfd6a3a168ad8f6bcece0e5bfc47f01e791ab8ff))
* **draw:** visibly render Mouth::mouth_open as growing ellipse ([#33](https://github.com/andymai/stackchan-kai/issues/33)) ([c21f9da](https://github.com/andymai/stackchan-kai/commit/c21f9da786472e94dbbe5e307dd052e172ebf646))
* emotion-coupled head motion (EmotionHead modifier) ([#4](https://github.com/andymai/stackchan-kai/issues/4)) ([3f197f1](https://github.com/andymai/stackchan-kai/commit/3f197f106527977da99cdd9ac75dab79462290c4))
* emotion-transition chirps — pickup, wake, low-battery audio cues ([#56](https://github.com/andymai/stackchan-kai/issues/56)) ([f097f8c](https://github.com/andymai/stackchan-kai/commit/f097f8c78f0e8299f088e8aac8180a17b89ad623))
* FT6336U tap-to-cycle emotion + shared I²C0 bus ([#15](https://github.com/andymai/stackchan-kai/issues/15)) ([b3fb8de](https://github.com/andymai/stackchan-kai/commit/b3fb8de289ac45f9c5537516a79c2dd2e3a4e6cb))
* low-battery hysteresis + USB-power aware override ([#54](https://github.com/andymai/stackchan-kai/issues/54)) ([0741a2e](https://github.com/andymai/stackchan-kai/commit/0741a2edcb9478c34e1fdb4e6739393cedfd2019))
* low-battery sleepy emotion — AXP2101 SoC reader, power task, modifier ([#52](https://github.com/andymai/stackchan-kai/issues/52)) ([d6add63](https://github.com/andymai/stackchan-kai/commit/d6add63fa0a16dc17cb5984c163eab80600f0885))
* pan/tilt servo head motion (+aw9523 extract, pca9685 driver) ([#2](https://github.com/andymai/stackchan-kai/issues/2)) ([9bc40a1](https://github.com/andymai/stackchan-kai/commit/9bc40a11b76108aad8a58af7a941a28fade5ea0c))
* power-button taps + IR NEC RemoteCommand modifier ([#19](https://github.com/andymai/stackchan-kai/issues/19)) ([f29c92e](https://github.com/andymai/stackchan-kai/commit/f29c92e7550fa450be0adb8b94a8b57801e18ddf))
* PY32 WS2812 LED ring + first output-sink path ([#20](https://github.com/andymai/stackchan-kai/issues/20)) ([14dcbd3](https://github.com/andymai/stackchan-kai/commit/14dcbd3be257175f0df973c5cb616c41d92c483b))
* servo position readback + calibration bench binary ([#11](https://github.com/andymai/stackchan-kai/issues/11)) ([890c8f8](https://github.com/andymai/stackchan-kai/commit/890c8f8232f80f3b6f861ded1ec2b2e386fbc31d))
* WakeOnVoice modifier — sustained mic activity wakes to Happy ([#55](https://github.com/andymai/stackchan-kai/issues/55)) ([c8729bf](https://github.com/andymai/stackchan-kai/commit/c8729bfbac9d78de54f20c64875d42e8544d0b8c))


### Bug Fixes

* tilt calibration for offset-encoder unit + asymmetric range support ([#47](https://github.com/andymai/stackchan-kai/issues/47)) ([52b8c4d](https://github.com/andymai/stackchan-kai/commit/52b8c4d47477baf776c82446d02431d08d24f941))

## [0.7.0](https://github.com/andymai/stackchan-kai/compare/v0.6.0...v0.7.0) (2026-04-25)


### Features

* BM8563 wall-clock + LTR-553 AmbientSleepy modifier ([#18](https://github.com/andymai/stackchan-kai/issues/18)) ([8405d0d](https://github.com/andymai/stackchan-kai/commit/8405d0d0f1619d400ff2bd1691995135f9c25316))
* BMI270 IMU + pickup-reaction modifier ([#17](https://github.com/andymai/stackchan-kai/issues/17)) ([8624ceb](https://github.com/andymai/stackchan-kai/commit/8624ceb92bcb355a28fa3c98ee6499171cb12a2b))
* BMM150 magnetometer (9-axis data path) ([#22](https://github.com/andymai/stackchan-kai/issues/22)) ([eea9212](https://github.com/andymai/stackchan-kai/commit/eea921233444d2efe68d7ee502e14be390778c20))
* **core:** MouthOpenAudio modifier + Mouth::mouth_open field ([#32](https://github.com/andymai/stackchan-kai/issues/32)) ([79020ed](https://github.com/andymai/stackchan-kai/commit/79020ed266f510b1bd2da1f7ecc01f8465105737))
* **core:** render Avatar onto any embedded-graphics DrawTarget ([a8d9b80](https://github.com/andymai/stackchan-kai/commit/a8d9b80a65ef8557d0cd829eadd873246b434138))
* **core:** wire Emotion into a style-field pipeline with eased transitions ([bfd6a3a](https://github.com/andymai/stackchan-kai/commit/bfd6a3a168ad8f6bcece0e5bfc47f01e791ab8ff))
* **draw:** visibly render Mouth::mouth_open as growing ellipse ([#33](https://github.com/andymai/stackchan-kai/issues/33)) ([c21f9da](https://github.com/andymai/stackchan-kai/commit/c21f9da786472e94dbbe5e307dd052e172ebf646))
* emotion-coupled head motion (EmotionHead modifier) ([#4](https://github.com/andymai/stackchan-kai/issues/4)) ([3f197f1](https://github.com/andymai/stackchan-kai/commit/3f197f106527977da99cdd9ac75dab79462290c4))
* emotion-transition chirps — pickup, wake, low-battery audio cues ([#56](https://github.com/andymai/stackchan-kai/issues/56)) ([f097f8c](https://github.com/andymai/stackchan-kai/commit/f097f8c78f0e8299f088e8aac8180a17b89ad623))
* FT6336U tap-to-cycle emotion + shared I²C0 bus ([#15](https://github.com/andymai/stackchan-kai/issues/15)) ([b3fb8de](https://github.com/andymai/stackchan-kai/commit/b3fb8de289ac45f9c5537516a79c2dd2e3a4e6cb))
* low-battery hysteresis + USB-power aware override ([#54](https://github.com/andymai/stackchan-kai/issues/54)) ([0741a2e](https://github.com/andymai/stackchan-kai/commit/0741a2edcb9478c34e1fdb4e6739393cedfd2019))
* low-battery sleepy emotion — AXP2101 SoC reader, power task, modifier ([#52](https://github.com/andymai/stackchan-kai/issues/52)) ([d6add63](https://github.com/andymai/stackchan-kai/commit/d6add63fa0a16dc17cb5984c163eab80600f0885))
* pan/tilt servo head motion (+aw9523 extract, pca9685 driver) ([#2](https://github.com/andymai/stackchan-kai/issues/2)) ([9bc40a1](https://github.com/andymai/stackchan-kai/commit/9bc40a11b76108aad8a58af7a941a28fade5ea0c))
* power-button taps + IR NEC RemoteCommand modifier ([#19](https://github.com/andymai/stackchan-kai/issues/19)) ([f29c92e](https://github.com/andymai/stackchan-kai/commit/f29c92e7550fa450be0adb8b94a8b57801e18ddf))
* PY32 WS2812 LED ring + first output-sink path ([#20](https://github.com/andymai/stackchan-kai/issues/20)) ([14dcbd3](https://github.com/andymai/stackchan-kai/commit/14dcbd3be257175f0df973c5cb616c41d92c483b))
* servo position readback + calibration bench binary ([#11](https://github.com/andymai/stackchan-kai/issues/11)) ([890c8f8](https://github.com/andymai/stackchan-kai/commit/890c8f8232f80f3b6f861ded1ec2b2e386fbc31d))
* WakeOnVoice modifier — sustained mic activity wakes to Happy ([#55](https://github.com/andymai/stackchan-kai/issues/55)) ([c8729bf](https://github.com/andymai/stackchan-kai/commit/c8729bfbac9d78de54f20c64875d42e8544d0b8c))


### Bug Fixes

* tilt calibration for offset-encoder unit + asymmetric range support ([#47](https://github.com/andymai/stackchan-kai/issues/47)) ([52b8c4d](https://github.com/andymai/stackchan-kai/commit/52b8c4d47477baf776c82446d02431d08d24f941))

## [0.6.0](https://github.com/andymai/stackchan-kai/compare/v0.5.0...v0.6.0) (2026-04-25)


### Features

* BM8563 wall-clock + LTR-553 AmbientSleepy modifier ([#18](https://github.com/andymai/stackchan-kai/issues/18)) ([8405d0d](https://github.com/andymai/stackchan-kai/commit/8405d0d0f1619d400ff2bd1691995135f9c25316))
* BMI270 IMU + pickup-reaction modifier ([#17](https://github.com/andymai/stackchan-kai/issues/17)) ([8624ceb](https://github.com/andymai/stackchan-kai/commit/8624ceb92bcb355a28fa3c98ee6499171cb12a2b))
* BMM150 magnetometer (9-axis data path) ([#22](https://github.com/andymai/stackchan-kai/issues/22)) ([eea9212](https://github.com/andymai/stackchan-kai/commit/eea921233444d2efe68d7ee502e14be390778c20))
* **core:** MouthOpenAudio modifier + Mouth::mouth_open field ([#32](https://github.com/andymai/stackchan-kai/issues/32)) ([79020ed](https://github.com/andymai/stackchan-kai/commit/79020ed266f510b1bd2da1f7ecc01f8465105737))
* **core:** render Avatar onto any embedded-graphics DrawTarget ([a8d9b80](https://github.com/andymai/stackchan-kai/commit/a8d9b80a65ef8557d0cd829eadd873246b434138))
* **core:** wire Emotion into a style-field pipeline with eased transitions ([bfd6a3a](https://github.com/andymai/stackchan-kai/commit/bfd6a3a168ad8f6bcece0e5bfc47f01e791ab8ff))
* **draw:** visibly render Mouth::mouth_open as growing ellipse ([#33](https://github.com/andymai/stackchan-kai/issues/33)) ([c21f9da](https://github.com/andymai/stackchan-kai/commit/c21f9da786472e94dbbe5e307dd052e172ebf646))
* emotion-coupled head motion (EmotionHead modifier) ([#4](https://github.com/andymai/stackchan-kai/issues/4)) ([3f197f1](https://github.com/andymai/stackchan-kai/commit/3f197f106527977da99cdd9ac75dab79462290c4))
* emotion-transition chirps — pickup, wake, low-battery audio cues ([#56](https://github.com/andymai/stackchan-kai/issues/56)) ([f097f8c](https://github.com/andymai/stackchan-kai/commit/f097f8c78f0e8299f088e8aac8180a17b89ad623))
* FT6336U tap-to-cycle emotion + shared I²C0 bus ([#15](https://github.com/andymai/stackchan-kai/issues/15)) ([b3fb8de](https://github.com/andymai/stackchan-kai/commit/b3fb8de289ac45f9c5537516a79c2dd2e3a4e6cb))
* low-battery hysteresis + USB-power aware override ([#54](https://github.com/andymai/stackchan-kai/issues/54)) ([0741a2e](https://github.com/andymai/stackchan-kai/commit/0741a2edcb9478c34e1fdb4e6739393cedfd2019))
* low-battery sleepy emotion — AXP2101 SoC reader, power task, modifier ([#52](https://github.com/andymai/stackchan-kai/issues/52)) ([d6add63](https://github.com/andymai/stackchan-kai/commit/d6add63fa0a16dc17cb5984c163eab80600f0885))
* pan/tilt servo head motion (+aw9523 extract, pca9685 driver) ([#2](https://github.com/andymai/stackchan-kai/issues/2)) ([9bc40a1](https://github.com/andymai/stackchan-kai/commit/9bc40a11b76108aad8a58af7a941a28fade5ea0c))
* power-button taps + IR NEC RemoteCommand modifier ([#19](https://github.com/andymai/stackchan-kai/issues/19)) ([f29c92e](https://github.com/andymai/stackchan-kai/commit/f29c92e7550fa450be0adb8b94a8b57801e18ddf))
* PY32 WS2812 LED ring + first output-sink path ([#20](https://github.com/andymai/stackchan-kai/issues/20)) ([14dcbd3](https://github.com/andymai/stackchan-kai/commit/14dcbd3be257175f0df973c5cb616c41d92c483b))
* servo position readback + calibration bench binary ([#11](https://github.com/andymai/stackchan-kai/issues/11)) ([890c8f8](https://github.com/andymai/stackchan-kai/commit/890c8f8232f80f3b6f861ded1ec2b2e386fbc31d))
* WakeOnVoice modifier — sustained mic activity wakes to Happy ([#55](https://github.com/andymai/stackchan-kai/issues/55)) ([c8729bf](https://github.com/andymai/stackchan-kai/commit/c8729bfbac9d78de54f20c64875d42e8544d0b8c))


### Bug Fixes

* tilt calibration for offset-encoder unit + asymmetric range support ([#47](https://github.com/andymai/stackchan-kai/issues/47)) ([52b8c4d](https://github.com/andymai/stackchan-kai/commit/52b8c4d47477baf776c82446d02431d08d24f941))

## [0.5.0](https://github.com/andymai/stackchan-kai/compare/v0.4.0...v0.5.0) (2026-04-25)


### Features

* BM8563 wall-clock + LTR-553 AmbientSleepy modifier ([#18](https://github.com/andymai/stackchan-kai/issues/18)) ([8405d0d](https://github.com/andymai/stackchan-kai/commit/8405d0d0f1619d400ff2bd1691995135f9c25316))
* BMI270 IMU + pickup-reaction modifier ([#17](https://github.com/andymai/stackchan-kai/issues/17)) ([8624ceb](https://github.com/andymai/stackchan-kai/commit/8624ceb92bcb355a28fa3c98ee6499171cb12a2b))
* BMM150 magnetometer (9-axis data path) ([#22](https://github.com/andymai/stackchan-kai/issues/22)) ([eea9212](https://github.com/andymai/stackchan-kai/commit/eea921233444d2efe68d7ee502e14be390778c20))
* **core:** MouthOpenAudio modifier + Mouth::mouth_open field ([#32](https://github.com/andymai/stackchan-kai/issues/32)) ([79020ed](https://github.com/andymai/stackchan-kai/commit/79020ed266f510b1bd2da1f7ecc01f8465105737))
* **core:** render Avatar onto any embedded-graphics DrawTarget ([a8d9b80](https://github.com/andymai/stackchan-kai/commit/a8d9b80a65ef8557d0cd829eadd873246b434138))
* **core:** wire Emotion into a style-field pipeline with eased transitions ([bfd6a3a](https://github.com/andymai/stackchan-kai/commit/bfd6a3a168ad8f6bcece0e5bfc47f01e791ab8ff))
* **draw:** visibly render Mouth::mouth_open as growing ellipse ([#33](https://github.com/andymai/stackchan-kai/issues/33)) ([c21f9da](https://github.com/andymai/stackchan-kai/commit/c21f9da786472e94dbbe5e307dd052e172ebf646))
* emotion-coupled head motion (EmotionHead modifier) ([#4](https://github.com/andymai/stackchan-kai/issues/4)) ([3f197f1](https://github.com/andymai/stackchan-kai/commit/3f197f106527977da99cdd9ac75dab79462290c4))
* FT6336U tap-to-cycle emotion + shared I²C0 bus ([#15](https://github.com/andymai/stackchan-kai/issues/15)) ([b3fb8de](https://github.com/andymai/stackchan-kai/commit/b3fb8de289ac45f9c5537516a79c2dd2e3a4e6cb))
* pan/tilt servo head motion (+aw9523 extract, pca9685 driver) ([#2](https://github.com/andymai/stackchan-kai/issues/2)) ([9bc40a1](https://github.com/andymai/stackchan-kai/commit/9bc40a11b76108aad8a58af7a941a28fade5ea0c))
* power-button taps + IR NEC RemoteCommand modifier ([#19](https://github.com/andymai/stackchan-kai/issues/19)) ([f29c92e](https://github.com/andymai/stackchan-kai/commit/f29c92e7550fa450be0adb8b94a8b57801e18ddf))
* PY32 WS2812 LED ring + first output-sink path ([#20](https://github.com/andymai/stackchan-kai/issues/20)) ([14dcbd3](https://github.com/andymai/stackchan-kai/commit/14dcbd3be257175f0df973c5cb616c41d92c483b))
* servo position readback + calibration bench binary ([#11](https://github.com/andymai/stackchan-kai/issues/11)) ([890c8f8](https://github.com/andymai/stackchan-kai/commit/890c8f8232f80f3b6f861ded1ec2b2e386fbc31d))


### Bug Fixes

* tilt calibration for offset-encoder unit + asymmetric range support ([#47](https://github.com/andymai/stackchan-kai/issues/47)) ([52b8c4d](https://github.com/andymai/stackchan-kai/commit/52b8c4d47477baf776c82446d02431d08d24f941))

## [0.4.0](https://github.com/andymai/stackchan-kai/compare/v0.3.0...v0.4.0) (2026-04-24)


### Features

* BM8563 wall-clock + LTR-553 AmbientSleepy modifier ([#18](https://github.com/andymai/stackchan-kai/issues/18)) ([a1f1af8](https://github.com/andymai/stackchan-kai/commit/a1f1af89d0409319cdf8cde60071dd8176ffae3b))
* BMI270 IMU + pickup-reaction modifier ([#17](https://github.com/andymai/stackchan-kai/issues/17)) ([3dae938](https://github.com/andymai/stackchan-kai/commit/3dae938089eaa76b28a5fc258e80a6f44999f4d9))
* BMM150 magnetometer (9-axis data path) ([#22](https://github.com/andymai/stackchan-kai/issues/22)) ([cdd2ff7](https://github.com/andymai/stackchan-kai/commit/cdd2ff79425afbf7f4d5eda89aa6e2c939859444))
* **core:** MouthOpenAudio modifier + Mouth::mouth_open field ([#32](https://github.com/andymai/stackchan-kai/issues/32)) ([8866fe6](https://github.com/andymai/stackchan-kai/commit/8866fe68f2f229ca238926bde28c503fcdf08e24))
* **core:** render Avatar onto any embedded-graphics DrawTarget ([a8d9b80](https://github.com/andymai/stackchan-kai/commit/a8d9b80a65ef8557d0cd829eadd873246b434138))
* **core:** wire Emotion into a style-field pipeline with eased transitions ([bfd6a3a](https://github.com/andymai/stackchan-kai/commit/bfd6a3a168ad8f6bcece0e5bfc47f01e791ab8ff))
* **draw:** visibly render Mouth::mouth_open as growing ellipse ([#33](https://github.com/andymai/stackchan-kai/issues/33)) ([78fcbdd](https://github.com/andymai/stackchan-kai/commit/78fcbddaebbaa4cbb8b7aae4ce7dde2371c4e227))
* emotion-coupled head motion (EmotionHead modifier) ([#4](https://github.com/andymai/stackchan-kai/issues/4)) ([f144bb8](https://github.com/andymai/stackchan-kai/commit/f144bb8dcb3f0e810137c0989ac22a0913067eda))
* FT6336U tap-to-cycle emotion + shared I²C0 bus ([#15](https://github.com/andymai/stackchan-kai/issues/15)) ([b724304](https://github.com/andymai/stackchan-kai/commit/b7243041f173deaa70d9cdf8b65f3a74430828c3))
* pan/tilt servo head motion (+aw9523 extract, pca9685 driver) ([#2](https://github.com/andymai/stackchan-kai/issues/2)) ([1caa3ce](https://github.com/andymai/stackchan-kai/commit/1caa3ced220093864b65f54dbba34cfe4a6a70c1))
* power-button taps + IR NEC RemoteCommand modifier ([#19](https://github.com/andymai/stackchan-kai/issues/19)) ([0542ced](https://github.com/andymai/stackchan-kai/commit/0542ced96f320938db52c58a436b988f654255f4))
* PY32 WS2812 LED ring + first output-sink path ([#20](https://github.com/andymai/stackchan-kai/issues/20)) ([fe5e598](https://github.com/andymai/stackchan-kai/commit/fe5e5989e6a8a2cee47e324a0ccf4479c336ba75))
* servo position readback + calibration bench binary ([#11](https://github.com/andymai/stackchan-kai/issues/11)) ([e5bf109](https://github.com/andymai/stackchan-kai/commit/e5bf10988ce5bf147b1cf2b5135874196d40255b))

## [0.3.0](https://github.com/andymai/stackchan-kai/compare/v0.2.0...v0.3.0) (2026-04-24)


### Features

* BM8563 wall-clock + LTR-553 AmbientSleepy modifier ([#18](https://github.com/andymai/stackchan-kai/issues/18)) ([a1f1af8](https://github.com/andymai/stackchan-kai/commit/a1f1af89d0409319cdf8cde60071dd8176ffae3b))
* BMI270 IMU + pickup-reaction modifier ([#17](https://github.com/andymai/stackchan-kai/issues/17)) ([3dae938](https://github.com/andymai/stackchan-kai/commit/3dae938089eaa76b28a5fc258e80a6f44999f4d9))
* FT6336U tap-to-cycle emotion + shared I²C0 bus ([#15](https://github.com/andymai/stackchan-kai/issues/15)) ([b724304](https://github.com/andymai/stackchan-kai/commit/b7243041f173deaa70d9cdf8b65f3a74430828c3))
* power-button taps + IR NEC RemoteCommand modifier ([#19](https://github.com/andymai/stackchan-kai/issues/19)) ([0542ced](https://github.com/andymai/stackchan-kai/commit/0542ced96f320938db52c58a436b988f654255f4))

## [0.2.0](https://github.com/andymai/stackchan-kai/compare/v0.1.0...v0.2.0) (2026-04-24)


### Features

* emotion-coupled head motion (EmotionHead modifier) ([#4](https://github.com/andymai/stackchan-kai/issues/4)) ([f144bb8](https://github.com/andymai/stackchan-kai/commit/f144bb8dcb3f0e810137c0989ac22a0913067eda))
* pan/tilt servo head motion (+aw9523 extract, pca9685 driver) ([#2](https://github.com/andymai/stackchan-kai/issues/2)) ([1caa3ce](https://github.com/andymai/stackchan-kai/commit/1caa3ced220093864b65f54dbba34cfe4a6a70c1))
* servo position readback + calibration bench binary ([#11](https://github.com/andymai/stackchan-kai/issues/11)) ([e5bf109](https://github.com/andymai/stackchan-kai/commit/e5bf10988ce5bf147b1cf2b5135874196d40255b))
