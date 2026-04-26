# Changelog

## [0.10.2](https://github.com/andymai/stackchan-kai/compare/stackchan-sim-v0.10.1...stackchan-sim-v0.10.2) (2026-04-26)

## [0.10.1](https://github.com/andymai/stackchan-kai/compare/v0.10.0...v0.10.1) (2026-04-26)

## [0.10.0](https://github.com/andymai/stackchan-kai/compare/v0.9.7...v0.10.0) (2026-04-26)


### ⚠ BREAKING CHANGES

* engine architecture — Entity + Director + Modifier/Skill ([#90](https://github.com/andymai/stackchan-kai/issues/90))


### Features

* **sim:** end-to-end Director-driven sim tests for new skills + modifiers (LookAtSound #92, BodyGesture #101, Petting #102, Handling #105, StartleOnLoud #106, AttentionFromTracking #111, head + eye reactions #112)


### Refactors

* **sim:** route firmware-stack tests through Director ([#96](https://github.com/andymai/stackchan-kai/issues/96))
* apply naming convention sweep across the inventory ([#108](https://github.com/andymai/stackchan-kai/issues/108))

## [0.9.0](https://github.com/andymai/stackchan-kai/compare/v0.8.0...v0.9.0) (2026-04-25)


### Features

* **core:** wire Emotion into a style-field pipeline with eased transitions ([bfd6a3a](https://github.com/andymai/stackchan-kai/commit/bfd6a3a168ad8f6bcece0e5bfc47f01e791ab8ff))
* **draw:** visibly render Mouth::mouth_open as growing ellipse ([#33](https://github.com/andymai/stackchan-kai/issues/33)) ([c21f9da](https://github.com/andymai/stackchan-kai/commit/c21f9da786472e94dbbe5e307dd052e172ebf646))
* pan/tilt servo head motion (+aw9523 extract, pca9685 driver) ([#2](https://github.com/andymai/stackchan-kai/issues/2)) ([9bc40a1](https://github.com/andymai/stackchan-kai/commit/9bc40a11b76108aad8a58af7a941a28fade5ea0c))
* PY32 WS2812 LED ring + first output-sink path ([#20](https://github.com/andymai/stackchan-kai/issues/20)) ([14dcbd3](https://github.com/andymai/stackchan-kai/commit/14dcbd3be257175f0df973c5cb616c41d92c483b))


### Bug Fixes

* tilt calibration for offset-encoder unit + asymmetric range support ([#47](https://github.com/andymai/stackchan-kai/issues/47)) ([52b8c4d](https://github.com/andymai/stackchan-kai/commit/52b8c4d47477baf776c82446d02431d08d24f941))

## [0.8.0](https://github.com/andymai/stackchan-kai/compare/v0.7.1...v0.8.0) (2026-04-25)


### Features

* **core:** wire Emotion into a style-field pipeline with eased transitions ([bfd6a3a](https://github.com/andymai/stackchan-kai/commit/bfd6a3a168ad8f6bcece0e5bfc47f01e791ab8ff))
* **draw:** visibly render Mouth::mouth_open as growing ellipse ([#33](https://github.com/andymai/stackchan-kai/issues/33)) ([c21f9da](https://github.com/andymai/stackchan-kai/commit/c21f9da786472e94dbbe5e307dd052e172ebf646))
* pan/tilt servo head motion (+aw9523 extract, pca9685 driver) ([#2](https://github.com/andymai/stackchan-kai/issues/2)) ([9bc40a1](https://github.com/andymai/stackchan-kai/commit/9bc40a11b76108aad8a58af7a941a28fade5ea0c))
* PY32 WS2812 LED ring + first output-sink path ([#20](https://github.com/andymai/stackchan-kai/issues/20)) ([14dcbd3](https://github.com/andymai/stackchan-kai/commit/14dcbd3be257175f0df973c5cb616c41d92c483b))


### Bug Fixes

* tilt calibration for offset-encoder unit + asymmetric range support ([#47](https://github.com/andymai/stackchan-kai/issues/47)) ([52b8c4d](https://github.com/andymai/stackchan-kai/commit/52b8c4d47477baf776c82446d02431d08d24f941))

## [0.7.1](https://github.com/andymai/stackchan-kai/compare/v0.7.0...v0.7.1) (2026-04-25)

## [0.7.0](https://github.com/andymai/stackchan-kai/compare/v0.6.0...v0.7.0) (2026-04-25)


### Features

* **core:** wire Emotion into a style-field pipeline with eased transitions ([bfd6a3a](https://github.com/andymai/stackchan-kai/commit/bfd6a3a168ad8f6bcece0e5bfc47f01e791ab8ff))
* **draw:** visibly render Mouth::mouth_open as growing ellipse ([#33](https://github.com/andymai/stackchan-kai/issues/33)) ([c21f9da](https://github.com/andymai/stackchan-kai/commit/c21f9da786472e94dbbe5e307dd052e172ebf646))
* pan/tilt servo head motion (+aw9523 extract, pca9685 driver) ([#2](https://github.com/andymai/stackchan-kai/issues/2)) ([9bc40a1](https://github.com/andymai/stackchan-kai/commit/9bc40a11b76108aad8a58af7a941a28fade5ea0c))
* PY32 WS2812 LED ring + first output-sink path ([#20](https://github.com/andymai/stackchan-kai/issues/20)) ([14dcbd3](https://github.com/andymai/stackchan-kai/commit/14dcbd3be257175f0df973c5cb616c41d92c483b))


### Bug Fixes

* tilt calibration for offset-encoder unit + asymmetric range support ([#47](https://github.com/andymai/stackchan-kai/issues/47)) ([52b8c4d](https://github.com/andymai/stackchan-kai/commit/52b8c4d47477baf776c82446d02431d08d24f941))

## [0.6.0](https://github.com/andymai/stackchan-kai/compare/v0.5.0...v0.6.0) (2026-04-25)


### Features

* **core:** wire Emotion into a style-field pipeline with eased transitions ([bfd6a3a](https://github.com/andymai/stackchan-kai/commit/bfd6a3a168ad8f6bcece0e5bfc47f01e791ab8ff))
* **draw:** visibly render Mouth::mouth_open as growing ellipse ([#33](https://github.com/andymai/stackchan-kai/issues/33)) ([c21f9da](https://github.com/andymai/stackchan-kai/commit/c21f9da786472e94dbbe5e307dd052e172ebf646))
* pan/tilt servo head motion (+aw9523 extract, pca9685 driver) ([#2](https://github.com/andymai/stackchan-kai/issues/2)) ([9bc40a1](https://github.com/andymai/stackchan-kai/commit/9bc40a11b76108aad8a58af7a941a28fade5ea0c))
* PY32 WS2812 LED ring + first output-sink path ([#20](https://github.com/andymai/stackchan-kai/issues/20)) ([14dcbd3](https://github.com/andymai/stackchan-kai/commit/14dcbd3be257175f0df973c5cb616c41d92c483b))


### Bug Fixes

* tilt calibration for offset-encoder unit + asymmetric range support ([#47](https://github.com/andymai/stackchan-kai/issues/47)) ([52b8c4d](https://github.com/andymai/stackchan-kai/commit/52b8c4d47477baf776c82446d02431d08d24f941))

## [0.5.0](https://github.com/andymai/stackchan-kai/compare/v0.4.0...v0.5.0) (2026-04-25)


### Features

* **core:** wire Emotion into a style-field pipeline with eased transitions ([bfd6a3a](https://github.com/andymai/stackchan-kai/commit/bfd6a3a168ad8f6bcece0e5bfc47f01e791ab8ff))
* **draw:** visibly render Mouth::mouth_open as growing ellipse ([#33](https://github.com/andymai/stackchan-kai/issues/33)) ([c21f9da](https://github.com/andymai/stackchan-kai/commit/c21f9da786472e94dbbe5e307dd052e172ebf646))
* pan/tilt servo head motion (+aw9523 extract, pca9685 driver) ([#2](https://github.com/andymai/stackchan-kai/issues/2)) ([9bc40a1](https://github.com/andymai/stackchan-kai/commit/9bc40a11b76108aad8a58af7a941a28fade5ea0c))
* PY32 WS2812 LED ring + first output-sink path ([#20](https://github.com/andymai/stackchan-kai/issues/20)) ([14dcbd3](https://github.com/andymai/stackchan-kai/commit/14dcbd3be257175f0df973c5cb616c41d92c483b))


### Bug Fixes

* tilt calibration for offset-encoder unit + asymmetric range support ([#47](https://github.com/andymai/stackchan-kai/issues/47)) ([52b8c4d](https://github.com/andymai/stackchan-kai/commit/52b8c4d47477baf776c82446d02431d08d24f941))

## [0.4.0](https://github.com/andymai/stackchan-kai/compare/v0.3.0...v0.4.0) (2026-04-25)


### Features

* **core:** wire Emotion into a style-field pipeline with eased transitions ([bfd6a3a](https://github.com/andymai/stackchan-kai/commit/bfd6a3a168ad8f6bcece0e5bfc47f01e791ab8ff))
* **draw:** visibly render Mouth::mouth_open as growing ellipse ([#33](https://github.com/andymai/stackchan-kai/issues/33)) ([c21f9da](https://github.com/andymai/stackchan-kai/commit/c21f9da786472e94dbbe5e307dd052e172ebf646))
* pan/tilt servo head motion (+aw9523 extract, pca9685 driver) ([#2](https://github.com/andymai/stackchan-kai/issues/2)) ([9bc40a1](https://github.com/andymai/stackchan-kai/commit/9bc40a11b76108aad8a58af7a941a28fade5ea0c))
* PY32 WS2812 LED ring + first output-sink path ([#20](https://github.com/andymai/stackchan-kai/issues/20)) ([14dcbd3](https://github.com/andymai/stackchan-kai/commit/14dcbd3be257175f0df973c5cb616c41d92c483b))


### Bug Fixes

* tilt calibration for offset-encoder unit + asymmetric range support ([#47](https://github.com/andymai/stackchan-kai/issues/47)) ([52b8c4d](https://github.com/andymai/stackchan-kai/commit/52b8c4d47477baf776c82446d02431d08d24f941))

## [0.3.0](https://github.com/andymai/stackchan-kai/compare/v0.2.1...v0.3.0) (2026-04-24)


### Features

* **core:** wire Emotion into a style-field pipeline with eased transitions ([bfd6a3a](https://github.com/andymai/stackchan-kai/commit/bfd6a3a168ad8f6bcece0e5bfc47f01e791ab8ff))
* **draw:** visibly render Mouth::mouth_open as growing ellipse ([#33](https://github.com/andymai/stackchan-kai/issues/33)) ([78fcbdd](https://github.com/andymai/stackchan-kai/commit/78fcbddaebbaa4cbb8b7aae4ce7dde2371c4e227))
* pan/tilt servo head motion (+aw9523 extract, pca9685 driver) ([#2](https://github.com/andymai/stackchan-kai/issues/2)) ([1caa3ce](https://github.com/andymai/stackchan-kai/commit/1caa3ced220093864b65f54dbba34cfe4a6a70c1))
* PY32 WS2812 LED ring + first output-sink path ([#20](https://github.com/andymai/stackchan-kai/issues/20)) ([fe5e598](https://github.com/andymai/stackchan-kai/commit/fe5e5989e6a8a2cee47e324a0ccf4479c336ba75))

## [0.2.1](https://github.com/andymai/stackchan-kai/compare/v0.2.0...v0.2.1) (2026-04-24)

## [0.2.0](https://github.com/andymai/stackchan-kai/compare/v0.1.0...v0.2.0) (2026-04-24)


### Features

* pan/tilt servo head motion (+aw9523 extract, pca9685 driver) ([#2](https://github.com/andymai/stackchan-kai/issues/2)) ([1caa3ce](https://github.com/andymai/stackchan-kai/commit/1caa3ced220093864b65f54dbba34cfe4a6a70c1))
