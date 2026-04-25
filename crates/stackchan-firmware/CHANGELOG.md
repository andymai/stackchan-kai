# Changelog

## [0.9.2](https://github.com/andymai/stackchan-kai/compare/v0.9.1...v0.9.2) (2026-04-25)

## [0.9.1](https://github.com/andymai/stackchan-kai/compare/v0.9.0...v0.9.1) (2026-04-25)

## [0.9.0](https://github.com/andymai/stackchan-kai/compare/v0.8.0...v0.9.0) (2026-04-25)


### Features

* **audio:** codec bring-up + audio signal plumbing (firmware task scaffold) ([#29](https://github.com/andymai/stackchan-kai/issues/29)) ([0ad42aa](https://github.com/andymai/stackchan-kai/commit/0ad42aa851978c008a9c0684445ece99654ee183))
* **audio:** I²S0 master + MCLK, codec bring-up inside task ([#30](https://github.com/andymai/stackchan-kai/issues/30)) ([e080ffd](https://github.com/andymai/stackchan-kai/commit/e080ffd82e3e2200e20736e6c35431bb23420535))
* **audio:** real AW88298 + ES7210 driver impls + control-path benches ([#28](https://github.com/andymai/stackchan-kai/issues/28)) ([bdd9038](https://github.com/andymai/stackchan-kai/commit/bdd9038653198e9df64d315951533ecae5378aae))
* block-grid motion tracker crate + bench example ([#63](https://github.com/andymai/stackchan-kai/issues/63)) ([9865af5](https://github.com/andymai/stackchan-kai/commit/9865af5f17143f7df73d9e88e91ef2aa8c44ef55))
* BM8563 wall-clock + LTR-553 AmbientSleepy modifier ([#18](https://github.com/andymai/stackchan-kai/issues/18)) ([8405d0d](https://github.com/andymai/stackchan-kai/commit/8405d0d0f1619d400ff2bd1691995135f9c25316))
* BMI270 IMU + pickup-reaction modifier ([#17](https://github.com/andymai/stackchan-kai/issues/17)) ([8624ceb](https://github.com/andymai/stackchan-kai/commit/8624ceb92bcb355a28fa3c98ee6499171cb12a2b))
* BMM150 magnetometer (9-axis data path) ([#22](https://github.com/andymai/stackchan-kai/issues/22)) ([eea9212](https://github.com/andymai/stackchan-kai/commit/eea921233444d2efe68d7ee502e14be390778c20))
* camera-preview mode — GC0308 + LCD_CAM ping-pong DMA, long-press toggle ([#60](https://github.com/andymai/stackchan-kai/issues/60)) ([5cb0b62](https://github.com/andymai/stackchan-kai/commit/5cb0b626823f5b06c8efafc25f2e55f4930dd915))
* **core:** MouthOpenAudio modifier + Mouth::mouth_open field ([#32](https://github.com/andymai/stackchan-kai/issues/32)) ([79020ed](https://github.com/andymai/stackchan-kai/commit/79020ed266f510b1bd2da1f7ecc01f8465105737))
* **core:** wire Emotion into a style-field pipeline with eased transitions ([bfd6a3a](https://github.com/andymai/stackchan-kai/commit/bfd6a3a168ad8f6bcece0e5bfc47f01e791ab8ff))
* **dx:** boot PING health check + boot-nod gesture + justfile ([#6](https://github.com/andymai/stackchan-kai/issues/6)) ([3955354](https://github.com/andymai/stackchan-kai/commit/3955354bcbbf904f5dc88f032dccf1327677399f))
* emotion-coupled head motion (EmotionHead modifier) ([#4](https://github.com/andymai/stackchan-kai/issues/4)) ([3f197f1](https://github.com/andymai/stackchan-kai/commit/3f197f106527977da99cdd9ac75dab79462290c4))
* emotion-transition chirps — pickup, wake, low-battery audio cues ([#56](https://github.com/andymai/stackchan-kai/issues/56)) ([f097f8c](https://github.com/andymai/stackchan-kai/commit/f097f8c78f0e8299f088e8aac8180a17b89ad623))
* **firmware:** 30 FPS render task with Blink + dirty-check ([46d52ed](https://github.com/andymai/stackchan-kai/commit/46d52ed49ad4024a5d5de8c91b8c2db7c340d326))
* **firmware:** add Breath + IdleDrift to the render stack ([7c1c1af](https://github.com/andymai/stackchan-kai/commit/7c1c1af413a478a2b895c3acc869cb12808516cb))
* **firmware:** audio TX clip queue + low-battery alert beep ([#53](https://github.com/andymai/stackchan-kai/issues/53)) ([c0ddc3f](https://github.com/andymai/stackchan-kai/commit/c0ddc3fdbc55bfe68f4f26b345a49f34cc158246))
* **firmware:** audio TX path — speaker bring-up + boot greeting + RX/TX join ([#51](https://github.com/andymai/stackchan-kai/issues/51)) ([b50beae](https://github.com/andymai/stackchan-kai/commit/b50beae5b02f7be69b3347595a66dff58450053d))
* **firmware:** audio_bench example — playlist of every clip ([#58](https://github.com/andymai/stackchan-kai/issues/58)) ([2d5564f](https://github.com/andymai/stackchan-kai/commit/2d5564f6dc7092b2f63d8852576507ac0f9c6340))
* **firmware:** double-buffer via PSRAM to eliminate direct-draw flicker ([940551c](https://github.com/andymai/stackchan-kai/commit/940551c5767d1221bbc354f1787e2e903dd83758))
* **firmware:** esp-rtos boot + AXP2101 LCD rails ([212dc5c](https://github.com/andymai/stackchan-kai/commit/212dc5c93a3a179bea956ebf1b7f538d3111f1e4))
* **firmware:** ILI9342C via mipidsi — one-shot Avatar render ([9265830](https://github.com/andymai/stackchan-kai/commit/926583005f80c4b4755f196707f7888d36cd5987))
* **firmware:** RMS sample loop — audio task → mouth pipeline live ([#48](https://github.com/andymai/stackchan-kai/issues/48)) ([c1eb250](https://github.com/andymai/stackchan-kai/commit/c1eb250440db2164588045daf951c5a8109f0338))
* **firmware:** time-of-day boot greeting via BM8563 RTC ([#57](https://github.com/andymai/stackchan-kai/issues/57)) ([84e7f15](https://github.com/andymai/stackchan-kai/commit/84e7f1575b15daf6814a6ecc5f2a28084ba998fb))
* FT6336U tap-to-cycle emotion + shared I²C0 bus ([#15](https://github.com/andymai/stackchan-kai/issues/15)) ([b3fb8de](https://github.com/andymai/stackchan-kai/commit/b3fb8de289ac45f9c5537516a79c2dd2e3a4e6cb))
* low-battery hysteresis + USB-power aware override ([#54](https://github.com/andymai/stackchan-kai/issues/54)) ([0741a2e](https://github.com/andymai/stackchan-kai/commit/0741a2edcb9478c34e1fdb4e6739393cedfd2019))
* low-battery sleepy emotion — AXP2101 SoC reader, power task, modifier ([#52](https://github.com/andymai/stackchan-kai/issues/52)) ([d6add63](https://github.com/andymai/stackchan-kai/commit/d6add63fa0a16dc17cb5984c163eab80600f0885))
* pan/tilt servo head motion (+aw9523 extract, pca9685 driver) ([#2](https://github.com/andymai/stackchan-kai/issues/2)) ([9bc40a1](https://github.com/andymai/stackchan-kai/commit/9bc40a11b76108aad8a58af7a941a28fade5ea0c))
* power-button taps + IR NEC RemoteCommand modifier ([#19](https://github.com/andymai/stackchan-kai/issues/19)) ([f29c92e](https://github.com/andymai/stackchan-kai/commit/f29c92e7550fa450be0adb8b94a8b57801e18ddf))
* PY32 WS2812 LED ring + first output-sink path ([#20](https://github.com/andymai/stackchan-kai/issues/20)) ([14dcbd3](https://github.com/andymai/stackchan-kai/commit/14dcbd3be257175f0df973c5cb616c41d92c483b))
* servo position readback + calibration bench binary ([#11](https://github.com/andymai/stackchan-kai/issues/11)) ([890c8f8](https://github.com/andymai/stackchan-kai/commit/890c8f8232f80f3b6f861ded1ec2b2e386fbc31d))
* swap PCA9685 for Feetech SCServo on UART1 (matches real HW) ([#5](https://github.com/andymai/stackchan-kai/issues/5)) ([1ff3376](https://github.com/andymai/stackchan-kai/commit/1ff3376440453924e64cb7497c1e3a8e698fdb48))
* WakeOnVoice modifier — sustained mic activity wakes to Happy ([#55](https://github.com/andymai/stackchan-kai/issues/55)) ([c8729bf](https://github.com/andymai/stackchan-kai/commit/c8729bfbac9d78de54f20c64875d42e8544d0b8c))


### Bug Fixes

* **axp2101:** apply full M5Unified CoreS3 init to stop idle shutdown ([f5bc712](https://github.com/andymai/stackchan-kai/commit/f5bc712073813630f3fe78d1331d918799e55f70))
* **es7210:** drop invented chip-ID check blocking bring-up ([#31](https://github.com/andymai/stackchan-kai/issues/31)) ([304ef58](https://github.com/andymai/stackchan-kai/commit/304ef582e025713f420ab30970781c9a9d11ae64))
* **firmware:** boot on CoreS3 hardware end-to-end ([dba4c89](https://github.com/andymai/stackchan-kai/commit/dba4c89b89ad27b8adc07143a8163607410efd69))
* **firmware:** enable SCServo torque after ping, restore yes-nod gesture ([5398094](https://github.com/andymai/stackchan-kai/commit/5398094e86512d6ff4f928c16471a96f65b0d4e4))
* **firmware:** enable SCServo torque after ping, restore yes-nod gesture ([daf03b7](https://github.com/andymai/stackchan-kai/commit/daf03b771aa6a72235773c4ee7eefc262901ed06))
* **firmware:** I²C 400 kHz, justfile `reattach` recipe + reliability notes ([#34](https://github.com/andymai/stackchan-kai/issues/34)) ([82a462a](https://github.com/andymai/stackchan-kai/commit/82a462a3d50c93207ce60a8b8af4ab12693c6615))
* **firmware:** quiet boot-time warnings, drop SCServo FIFO-overflow spam ([10fd917](https://github.com/andymai/stackchan-kai/commit/10fd917651c66e6c3dcda939654f238e7b0e68ec))
* **firmware:** quiet boot-time warnings, drop SCServo FIFO-overflow spam ([28015fd](https://github.com/andymai/stackchan-kai/commit/28015fdb76c7523c249b4cbff239de33ba692589))
* **firmware:** restore LCD backlight + full AW9523 init on CoreS3 ([31ea98e](https://github.com/andymai/stackchan-kai/commit/31ea98e0d49a9329e72bf35357e227301492e23a))
* **firmware:** retry BMI270 init on I²C timeout, log SCServo angle limits ([0d477e5](https://github.com/andymai/stackchan-kai/commit/0d477e5e2c609e35df8df4279be9083280f56949))
* **firmware:** retry BMI270 init on I²C timeout, log SCServo angle limits ([fdbdbda](https://github.com/andymai/stackchan-kai/commit/fdbdbdaa41c826188fd4b3b37b85ffec9cff2bc1))
* **firmware:** revert I²C to 100 kHz, reduce boot-nod tilt amplitude ([f42315c](https://github.com/andymai/stackchan-kai/commit/f42315cd105f24396f3948c14be1b10e3d6d14f9))
* **firmware:** revert I²C to 100 kHz, reduce boot-nod tilt amplitude ([cb74dca](https://github.com/andymai/stackchan-kai/commit/cb74dcad6caa2c74b7ae1d3434dd9c98f6cd992d))
* **firmware:** satisfy pedantic clippy lints blocking CI ([0a37661](https://github.com/andymai/stackchan-kai/commit/0a37661ab74f4081f3ce5e4ba015236b5bce76c4))
* tilt calibration for offset-encoder unit + asymmetric range support ([#47](https://github.com/andymai/stackchan-kai/issues/47)) ([52b8c4d](https://github.com/andymai/stackchan-kai/commit/52b8c4d47477baf776c82446d02431d08d24f941))

## [0.8.0](https://github.com/andymai/stackchan-kai/compare/v0.7.0...v0.8.0) (2026-04-25)


### Features

* **audio:** codec bring-up + audio signal plumbing (firmware task scaffold) ([#29](https://github.com/andymai/stackchan-kai/issues/29)) ([0ad42aa](https://github.com/andymai/stackchan-kai/commit/0ad42aa851978c008a9c0684445ece99654ee183))
* **audio:** I²S0 master + MCLK, codec bring-up inside task ([#30](https://github.com/andymai/stackchan-kai/issues/30)) ([e080ffd](https://github.com/andymai/stackchan-kai/commit/e080ffd82e3e2200e20736e6c35431bb23420535))
* **audio:** real AW88298 + ES7210 driver impls + control-path benches ([#28](https://github.com/andymai/stackchan-kai/issues/28)) ([bdd9038](https://github.com/andymai/stackchan-kai/commit/bdd9038653198e9df64d315951533ecae5378aae))
* BM8563 wall-clock + LTR-553 AmbientSleepy modifier ([#18](https://github.com/andymai/stackchan-kai/issues/18)) ([8405d0d](https://github.com/andymai/stackchan-kai/commit/8405d0d0f1619d400ff2bd1691995135f9c25316))
* BMI270 IMU + pickup-reaction modifier ([#17](https://github.com/andymai/stackchan-kai/issues/17)) ([8624ceb](https://github.com/andymai/stackchan-kai/commit/8624ceb92bcb355a28fa3c98ee6499171cb12a2b))
* BMM150 magnetometer (9-axis data path) ([#22](https://github.com/andymai/stackchan-kai/issues/22)) ([eea9212](https://github.com/andymai/stackchan-kai/commit/eea921233444d2efe68d7ee502e14be390778c20))
* camera-preview mode — GC0308 + LCD_CAM ping-pong DMA, long-press toggle ([#60](https://github.com/andymai/stackchan-kai/issues/60)) ([5cb0b62](https://github.com/andymai/stackchan-kai/commit/5cb0b626823f5b06c8efafc25f2e55f4930dd915))
* **core:** MouthOpenAudio modifier + Mouth::mouth_open field ([#32](https://github.com/andymai/stackchan-kai/issues/32)) ([79020ed](https://github.com/andymai/stackchan-kai/commit/79020ed266f510b1bd2da1f7ecc01f8465105737))
* **core:** wire Emotion into a style-field pipeline with eased transitions ([bfd6a3a](https://github.com/andymai/stackchan-kai/commit/bfd6a3a168ad8f6bcece0e5bfc47f01e791ab8ff))
* **dx:** boot PING health check + boot-nod gesture + justfile ([#6](https://github.com/andymai/stackchan-kai/issues/6)) ([3955354](https://github.com/andymai/stackchan-kai/commit/3955354bcbbf904f5dc88f032dccf1327677399f))
* emotion-coupled head motion (EmotionHead modifier) ([#4](https://github.com/andymai/stackchan-kai/issues/4)) ([3f197f1](https://github.com/andymai/stackchan-kai/commit/3f197f106527977da99cdd9ac75dab79462290c4))
* emotion-transition chirps — pickup, wake, low-battery audio cues ([#56](https://github.com/andymai/stackchan-kai/issues/56)) ([f097f8c](https://github.com/andymai/stackchan-kai/commit/f097f8c78f0e8299f088e8aac8180a17b89ad623))
* **firmware:** 30 FPS render task with Blink + dirty-check ([46d52ed](https://github.com/andymai/stackchan-kai/commit/46d52ed49ad4024a5d5de8c91b8c2db7c340d326))
* **firmware:** add Breath + IdleDrift to the render stack ([7c1c1af](https://github.com/andymai/stackchan-kai/commit/7c1c1af413a478a2b895c3acc869cb12808516cb))
* **firmware:** audio TX clip queue + low-battery alert beep ([#53](https://github.com/andymai/stackchan-kai/issues/53)) ([c0ddc3f](https://github.com/andymai/stackchan-kai/commit/c0ddc3fdbc55bfe68f4f26b345a49f34cc158246))
* **firmware:** audio TX path — speaker bring-up + boot greeting + RX/TX join ([#51](https://github.com/andymai/stackchan-kai/issues/51)) ([b50beae](https://github.com/andymai/stackchan-kai/commit/b50beae5b02f7be69b3347595a66dff58450053d))
* **firmware:** audio_bench example — playlist of every clip ([#58](https://github.com/andymai/stackchan-kai/issues/58)) ([2d5564f](https://github.com/andymai/stackchan-kai/commit/2d5564f6dc7092b2f63d8852576507ac0f9c6340))
* **firmware:** double-buffer via PSRAM to eliminate direct-draw flicker ([940551c](https://github.com/andymai/stackchan-kai/commit/940551c5767d1221bbc354f1787e2e903dd83758))
* **firmware:** esp-rtos boot + AXP2101 LCD rails ([212dc5c](https://github.com/andymai/stackchan-kai/commit/212dc5c93a3a179bea956ebf1b7f538d3111f1e4))
* **firmware:** ILI9342C via mipidsi — one-shot Avatar render ([9265830](https://github.com/andymai/stackchan-kai/commit/926583005f80c4b4755f196707f7888d36cd5987))
* **firmware:** RMS sample loop — audio task → mouth pipeline live ([#48](https://github.com/andymai/stackchan-kai/issues/48)) ([c1eb250](https://github.com/andymai/stackchan-kai/commit/c1eb250440db2164588045daf951c5a8109f0338))
* **firmware:** time-of-day boot greeting via BM8563 RTC ([#57](https://github.com/andymai/stackchan-kai/issues/57)) ([84e7f15](https://github.com/andymai/stackchan-kai/commit/84e7f1575b15daf6814a6ecc5f2a28084ba998fb))
* FT6336U tap-to-cycle emotion + shared I²C0 bus ([#15](https://github.com/andymai/stackchan-kai/issues/15)) ([b3fb8de](https://github.com/andymai/stackchan-kai/commit/b3fb8de289ac45f9c5537516a79c2dd2e3a4e6cb))
* low-battery hysteresis + USB-power aware override ([#54](https://github.com/andymai/stackchan-kai/issues/54)) ([0741a2e](https://github.com/andymai/stackchan-kai/commit/0741a2edcb9478c34e1fdb4e6739393cedfd2019))
* low-battery sleepy emotion — AXP2101 SoC reader, power task, modifier ([#52](https://github.com/andymai/stackchan-kai/issues/52)) ([d6add63](https://github.com/andymai/stackchan-kai/commit/d6add63fa0a16dc17cb5984c163eab80600f0885))
* pan/tilt servo head motion (+aw9523 extract, pca9685 driver) ([#2](https://github.com/andymai/stackchan-kai/issues/2)) ([9bc40a1](https://github.com/andymai/stackchan-kai/commit/9bc40a11b76108aad8a58af7a941a28fade5ea0c))
* power-button taps + IR NEC RemoteCommand modifier ([#19](https://github.com/andymai/stackchan-kai/issues/19)) ([f29c92e](https://github.com/andymai/stackchan-kai/commit/f29c92e7550fa450be0adb8b94a8b57801e18ddf))
* PY32 WS2812 LED ring + first output-sink path ([#20](https://github.com/andymai/stackchan-kai/issues/20)) ([14dcbd3](https://github.com/andymai/stackchan-kai/commit/14dcbd3be257175f0df973c5cb616c41d92c483b))
* servo position readback + calibration bench binary ([#11](https://github.com/andymai/stackchan-kai/issues/11)) ([890c8f8](https://github.com/andymai/stackchan-kai/commit/890c8f8232f80f3b6f861ded1ec2b2e386fbc31d))
* swap PCA9685 for Feetech SCServo on UART1 (matches real HW) ([#5](https://github.com/andymai/stackchan-kai/issues/5)) ([1ff3376](https://github.com/andymai/stackchan-kai/commit/1ff3376440453924e64cb7497c1e3a8e698fdb48))
* WakeOnVoice modifier — sustained mic activity wakes to Happy ([#55](https://github.com/andymai/stackchan-kai/issues/55)) ([c8729bf](https://github.com/andymai/stackchan-kai/commit/c8729bfbac9d78de54f20c64875d42e8544d0b8c))


### Bug Fixes

* **axp2101:** apply full M5Unified CoreS3 init to stop idle shutdown ([f5bc712](https://github.com/andymai/stackchan-kai/commit/f5bc712073813630f3fe78d1331d918799e55f70))
* **es7210:** drop invented chip-ID check blocking bring-up ([#31](https://github.com/andymai/stackchan-kai/issues/31)) ([304ef58](https://github.com/andymai/stackchan-kai/commit/304ef582e025713f420ab30970781c9a9d11ae64))
* **firmware:** boot on CoreS3 hardware end-to-end ([dba4c89](https://github.com/andymai/stackchan-kai/commit/dba4c89b89ad27b8adc07143a8163607410efd69))
* **firmware:** enable SCServo torque after ping, restore yes-nod gesture ([5398094](https://github.com/andymai/stackchan-kai/commit/5398094e86512d6ff4f928c16471a96f65b0d4e4))
* **firmware:** enable SCServo torque after ping, restore yes-nod gesture ([daf03b7](https://github.com/andymai/stackchan-kai/commit/daf03b771aa6a72235773c4ee7eefc262901ed06))
* **firmware:** I²C 400 kHz, justfile `reattach` recipe + reliability notes ([#34](https://github.com/andymai/stackchan-kai/issues/34)) ([82a462a](https://github.com/andymai/stackchan-kai/commit/82a462a3d50c93207ce60a8b8af4ab12693c6615))
* **firmware:** quiet boot-time warnings, drop SCServo FIFO-overflow spam ([10fd917](https://github.com/andymai/stackchan-kai/commit/10fd917651c66e6c3dcda939654f238e7b0e68ec))
* **firmware:** quiet boot-time warnings, drop SCServo FIFO-overflow spam ([28015fd](https://github.com/andymai/stackchan-kai/commit/28015fdb76c7523c249b4cbff239de33ba692589))
* **firmware:** restore LCD backlight + full AW9523 init on CoreS3 ([31ea98e](https://github.com/andymai/stackchan-kai/commit/31ea98e0d49a9329e72bf35357e227301492e23a))
* **firmware:** retry BMI270 init on I²C timeout, log SCServo angle limits ([0d477e5](https://github.com/andymai/stackchan-kai/commit/0d477e5e2c609e35df8df4279be9083280f56949))
* **firmware:** retry BMI270 init on I²C timeout, log SCServo angle limits ([fdbdbda](https://github.com/andymai/stackchan-kai/commit/fdbdbdaa41c826188fd4b3b37b85ffec9cff2bc1))
* **firmware:** revert I²C to 100 kHz, reduce boot-nod tilt amplitude ([f42315c](https://github.com/andymai/stackchan-kai/commit/f42315cd105f24396f3948c14be1b10e3d6d14f9))
* **firmware:** revert I²C to 100 kHz, reduce boot-nod tilt amplitude ([cb74dca](https://github.com/andymai/stackchan-kai/commit/cb74dcad6caa2c74b7ae1d3434dd9c98f6cd992d))
* **firmware:** satisfy pedantic clippy lints blocking CI ([0a37661](https://github.com/andymai/stackchan-kai/commit/0a37661ab74f4081f3ce5e4ba015236b5bce76c4))
* tilt calibration for offset-encoder unit + asymmetric range support ([#47](https://github.com/andymai/stackchan-kai/issues/47)) ([52b8c4d](https://github.com/andymai/stackchan-kai/commit/52b8c4d47477baf776c82446d02431d08d24f941))

## [0.7.0](https://github.com/andymai/stackchan-kai/compare/v0.6.0...v0.7.0) (2026-04-25)


### Features

* **audio:** codec bring-up + audio signal plumbing (firmware task scaffold) ([#29](https://github.com/andymai/stackchan-kai/issues/29)) ([0ad42aa](https://github.com/andymai/stackchan-kai/commit/0ad42aa851978c008a9c0684445ece99654ee183))
* **audio:** I²S0 master + MCLK, codec bring-up inside task ([#30](https://github.com/andymai/stackchan-kai/issues/30)) ([e080ffd](https://github.com/andymai/stackchan-kai/commit/e080ffd82e3e2200e20736e6c35431bb23420535))
* **audio:** real AW88298 + ES7210 driver impls + control-path benches ([#28](https://github.com/andymai/stackchan-kai/issues/28)) ([bdd9038](https://github.com/andymai/stackchan-kai/commit/bdd9038653198e9df64d315951533ecae5378aae))
* BM8563 wall-clock + LTR-553 AmbientSleepy modifier ([#18](https://github.com/andymai/stackchan-kai/issues/18)) ([8405d0d](https://github.com/andymai/stackchan-kai/commit/8405d0d0f1619d400ff2bd1691995135f9c25316))
* BMI270 IMU + pickup-reaction modifier ([#17](https://github.com/andymai/stackchan-kai/issues/17)) ([8624ceb](https://github.com/andymai/stackchan-kai/commit/8624ceb92bcb355a28fa3c98ee6499171cb12a2b))
* BMM150 magnetometer (9-axis data path) ([#22](https://github.com/andymai/stackchan-kai/issues/22)) ([eea9212](https://github.com/andymai/stackchan-kai/commit/eea921233444d2efe68d7ee502e14be390778c20))
* camera-preview mode — GC0308 + LCD_CAM ping-pong DMA, long-press toggle ([#60](https://github.com/andymai/stackchan-kai/issues/60)) ([5cb0b62](https://github.com/andymai/stackchan-kai/commit/5cb0b626823f5b06c8efafc25f2e55f4930dd915))
* **core:** MouthOpenAudio modifier + Mouth::mouth_open field ([#32](https://github.com/andymai/stackchan-kai/issues/32)) ([79020ed](https://github.com/andymai/stackchan-kai/commit/79020ed266f510b1bd2da1f7ecc01f8465105737))
* **core:** wire Emotion into a style-field pipeline with eased transitions ([bfd6a3a](https://github.com/andymai/stackchan-kai/commit/bfd6a3a168ad8f6bcece0e5bfc47f01e791ab8ff))
* **dx:** boot PING health check + boot-nod gesture + justfile ([#6](https://github.com/andymai/stackchan-kai/issues/6)) ([3955354](https://github.com/andymai/stackchan-kai/commit/3955354bcbbf904f5dc88f032dccf1327677399f))
* emotion-coupled head motion (EmotionHead modifier) ([#4](https://github.com/andymai/stackchan-kai/issues/4)) ([3f197f1](https://github.com/andymai/stackchan-kai/commit/3f197f106527977da99cdd9ac75dab79462290c4))
* emotion-transition chirps — pickup, wake, low-battery audio cues ([#56](https://github.com/andymai/stackchan-kai/issues/56)) ([f097f8c](https://github.com/andymai/stackchan-kai/commit/f097f8c78f0e8299f088e8aac8180a17b89ad623))
* **firmware:** 30 FPS render task with Blink + dirty-check ([46d52ed](https://github.com/andymai/stackchan-kai/commit/46d52ed49ad4024a5d5de8c91b8c2db7c340d326))
* **firmware:** add Breath + IdleDrift to the render stack ([7c1c1af](https://github.com/andymai/stackchan-kai/commit/7c1c1af413a478a2b895c3acc869cb12808516cb))
* **firmware:** audio TX clip queue + low-battery alert beep ([#53](https://github.com/andymai/stackchan-kai/issues/53)) ([c0ddc3f](https://github.com/andymai/stackchan-kai/commit/c0ddc3fdbc55bfe68f4f26b345a49f34cc158246))
* **firmware:** audio TX path — speaker bring-up + boot greeting + RX/TX join ([#51](https://github.com/andymai/stackchan-kai/issues/51)) ([b50beae](https://github.com/andymai/stackchan-kai/commit/b50beae5b02f7be69b3347595a66dff58450053d))
* **firmware:** audio_bench example — playlist of every clip ([#58](https://github.com/andymai/stackchan-kai/issues/58)) ([2d5564f](https://github.com/andymai/stackchan-kai/commit/2d5564f6dc7092b2f63d8852576507ac0f9c6340))
* **firmware:** double-buffer via PSRAM to eliminate direct-draw flicker ([940551c](https://github.com/andymai/stackchan-kai/commit/940551c5767d1221bbc354f1787e2e903dd83758))
* **firmware:** esp-rtos boot + AXP2101 LCD rails ([212dc5c](https://github.com/andymai/stackchan-kai/commit/212dc5c93a3a179bea956ebf1b7f538d3111f1e4))
* **firmware:** ILI9342C via mipidsi — one-shot Avatar render ([9265830](https://github.com/andymai/stackchan-kai/commit/926583005f80c4b4755f196707f7888d36cd5987))
* **firmware:** RMS sample loop — audio task → mouth pipeline live ([#48](https://github.com/andymai/stackchan-kai/issues/48)) ([c1eb250](https://github.com/andymai/stackchan-kai/commit/c1eb250440db2164588045daf951c5a8109f0338))
* **firmware:** time-of-day boot greeting via BM8563 RTC ([#57](https://github.com/andymai/stackchan-kai/issues/57)) ([84e7f15](https://github.com/andymai/stackchan-kai/commit/84e7f1575b15daf6814a6ecc5f2a28084ba998fb))
* FT6336U tap-to-cycle emotion + shared I²C0 bus ([#15](https://github.com/andymai/stackchan-kai/issues/15)) ([b3fb8de](https://github.com/andymai/stackchan-kai/commit/b3fb8de289ac45f9c5537516a79c2dd2e3a4e6cb))
* low-battery hysteresis + USB-power aware override ([#54](https://github.com/andymai/stackchan-kai/issues/54)) ([0741a2e](https://github.com/andymai/stackchan-kai/commit/0741a2edcb9478c34e1fdb4e6739393cedfd2019))
* low-battery sleepy emotion — AXP2101 SoC reader, power task, modifier ([#52](https://github.com/andymai/stackchan-kai/issues/52)) ([d6add63](https://github.com/andymai/stackchan-kai/commit/d6add63fa0a16dc17cb5984c163eab80600f0885))
* pan/tilt servo head motion (+aw9523 extract, pca9685 driver) ([#2](https://github.com/andymai/stackchan-kai/issues/2)) ([9bc40a1](https://github.com/andymai/stackchan-kai/commit/9bc40a11b76108aad8a58af7a941a28fade5ea0c))
* power-button taps + IR NEC RemoteCommand modifier ([#19](https://github.com/andymai/stackchan-kai/issues/19)) ([f29c92e](https://github.com/andymai/stackchan-kai/commit/f29c92e7550fa450be0adb8b94a8b57801e18ddf))
* PY32 WS2812 LED ring + first output-sink path ([#20](https://github.com/andymai/stackchan-kai/issues/20)) ([14dcbd3](https://github.com/andymai/stackchan-kai/commit/14dcbd3be257175f0df973c5cb616c41d92c483b))
* servo position readback + calibration bench binary ([#11](https://github.com/andymai/stackchan-kai/issues/11)) ([890c8f8](https://github.com/andymai/stackchan-kai/commit/890c8f8232f80f3b6f861ded1ec2b2e386fbc31d))
* swap PCA9685 for Feetech SCServo on UART1 (matches real HW) ([#5](https://github.com/andymai/stackchan-kai/issues/5)) ([1ff3376](https://github.com/andymai/stackchan-kai/commit/1ff3376440453924e64cb7497c1e3a8e698fdb48))
* WakeOnVoice modifier — sustained mic activity wakes to Happy ([#55](https://github.com/andymai/stackchan-kai/issues/55)) ([c8729bf](https://github.com/andymai/stackchan-kai/commit/c8729bfbac9d78de54f20c64875d42e8544d0b8c))


### Bug Fixes

* **axp2101:** apply full M5Unified CoreS3 init to stop idle shutdown ([f5bc712](https://github.com/andymai/stackchan-kai/commit/f5bc712073813630f3fe78d1331d918799e55f70))
* **es7210:** drop invented chip-ID check blocking bring-up ([#31](https://github.com/andymai/stackchan-kai/issues/31)) ([304ef58](https://github.com/andymai/stackchan-kai/commit/304ef582e025713f420ab30970781c9a9d11ae64))
* **firmware:** boot on CoreS3 hardware end-to-end ([dba4c89](https://github.com/andymai/stackchan-kai/commit/dba4c89b89ad27b8adc07143a8163607410efd69))
* **firmware:** enable SCServo torque after ping, restore yes-nod gesture ([5398094](https://github.com/andymai/stackchan-kai/commit/5398094e86512d6ff4f928c16471a96f65b0d4e4))
* **firmware:** enable SCServo torque after ping, restore yes-nod gesture ([daf03b7](https://github.com/andymai/stackchan-kai/commit/daf03b771aa6a72235773c4ee7eefc262901ed06))
* **firmware:** I²C 400 kHz, justfile `reattach` recipe + reliability notes ([#34](https://github.com/andymai/stackchan-kai/issues/34)) ([82a462a](https://github.com/andymai/stackchan-kai/commit/82a462a3d50c93207ce60a8b8af4ab12693c6615))
* **firmware:** quiet boot-time warnings, drop SCServo FIFO-overflow spam ([10fd917](https://github.com/andymai/stackchan-kai/commit/10fd917651c66e6c3dcda939654f238e7b0e68ec))
* **firmware:** quiet boot-time warnings, drop SCServo FIFO-overflow spam ([28015fd](https://github.com/andymai/stackchan-kai/commit/28015fdb76c7523c249b4cbff239de33ba692589))
* **firmware:** restore LCD backlight + full AW9523 init on CoreS3 ([31ea98e](https://github.com/andymai/stackchan-kai/commit/31ea98e0d49a9329e72bf35357e227301492e23a))
* **firmware:** retry BMI270 init on I²C timeout, log SCServo angle limits ([0d477e5](https://github.com/andymai/stackchan-kai/commit/0d477e5e2c609e35df8df4279be9083280f56949))
* **firmware:** retry BMI270 init on I²C timeout, log SCServo angle limits ([fdbdbda](https://github.com/andymai/stackchan-kai/commit/fdbdbdaa41c826188fd4b3b37b85ffec9cff2bc1))
* **firmware:** revert I²C to 100 kHz, reduce boot-nod tilt amplitude ([f42315c](https://github.com/andymai/stackchan-kai/commit/f42315cd105f24396f3948c14be1b10e3d6d14f9))
* **firmware:** revert I²C to 100 kHz, reduce boot-nod tilt amplitude ([cb74dca](https://github.com/andymai/stackchan-kai/commit/cb74dcad6caa2c74b7ae1d3434dd9c98f6cd992d))
* **firmware:** satisfy pedantic clippy lints blocking CI ([0a37661](https://github.com/andymai/stackchan-kai/commit/0a37661ab74f4081f3ce5e4ba015236b5bce76c4))
* tilt calibration for offset-encoder unit + asymmetric range support ([#47](https://github.com/andymai/stackchan-kai/issues/47)) ([52b8c4d](https://github.com/andymai/stackchan-kai/commit/52b8c4d47477baf776c82446d02431d08d24f941))

## [0.6.0](https://github.com/andymai/stackchan-kai/compare/v0.5.0...v0.6.0) (2026-04-25)


### Features

* **audio:** codec bring-up + audio signal plumbing (firmware task scaffold) ([#29](https://github.com/andymai/stackchan-kai/issues/29)) ([0ad42aa](https://github.com/andymai/stackchan-kai/commit/0ad42aa851978c008a9c0684445ece99654ee183))
* **audio:** I²S0 master + MCLK, codec bring-up inside task ([#30](https://github.com/andymai/stackchan-kai/issues/30)) ([e080ffd](https://github.com/andymai/stackchan-kai/commit/e080ffd82e3e2200e20736e6c35431bb23420535))
* **audio:** real AW88298 + ES7210 driver impls + control-path benches ([#28](https://github.com/andymai/stackchan-kai/issues/28)) ([bdd9038](https://github.com/andymai/stackchan-kai/commit/bdd9038653198e9df64d315951533ecae5378aae))
* BM8563 wall-clock + LTR-553 AmbientSleepy modifier ([#18](https://github.com/andymai/stackchan-kai/issues/18)) ([8405d0d](https://github.com/andymai/stackchan-kai/commit/8405d0d0f1619d400ff2bd1691995135f9c25316))
* BMI270 IMU + pickup-reaction modifier ([#17](https://github.com/andymai/stackchan-kai/issues/17)) ([8624ceb](https://github.com/andymai/stackchan-kai/commit/8624ceb92bcb355a28fa3c98ee6499171cb12a2b))
* BMM150 magnetometer (9-axis data path) ([#22](https://github.com/andymai/stackchan-kai/issues/22)) ([eea9212](https://github.com/andymai/stackchan-kai/commit/eea921233444d2efe68d7ee502e14be390778c20))
* **core:** MouthOpenAudio modifier + Mouth::mouth_open field ([#32](https://github.com/andymai/stackchan-kai/issues/32)) ([79020ed](https://github.com/andymai/stackchan-kai/commit/79020ed266f510b1bd2da1f7ecc01f8465105737))
* **core:** wire Emotion into a style-field pipeline with eased transitions ([bfd6a3a](https://github.com/andymai/stackchan-kai/commit/bfd6a3a168ad8f6bcece0e5bfc47f01e791ab8ff))
* **dx:** boot PING health check + boot-nod gesture + justfile ([#6](https://github.com/andymai/stackchan-kai/issues/6)) ([3955354](https://github.com/andymai/stackchan-kai/commit/3955354bcbbf904f5dc88f032dccf1327677399f))
* emotion-coupled head motion (EmotionHead modifier) ([#4](https://github.com/andymai/stackchan-kai/issues/4)) ([3f197f1](https://github.com/andymai/stackchan-kai/commit/3f197f106527977da99cdd9ac75dab79462290c4))
* emotion-transition chirps — pickup, wake, low-battery audio cues ([#56](https://github.com/andymai/stackchan-kai/issues/56)) ([f097f8c](https://github.com/andymai/stackchan-kai/commit/f097f8c78f0e8299f088e8aac8180a17b89ad623))
* **firmware:** 30 FPS render task with Blink + dirty-check ([46d52ed](https://github.com/andymai/stackchan-kai/commit/46d52ed49ad4024a5d5de8c91b8c2db7c340d326))
* **firmware:** add Breath + IdleDrift to the render stack ([7c1c1af](https://github.com/andymai/stackchan-kai/commit/7c1c1af413a478a2b895c3acc869cb12808516cb))
* **firmware:** audio TX clip queue + low-battery alert beep ([#53](https://github.com/andymai/stackchan-kai/issues/53)) ([c0ddc3f](https://github.com/andymai/stackchan-kai/commit/c0ddc3fdbc55bfe68f4f26b345a49f34cc158246))
* **firmware:** audio TX path — speaker bring-up + boot greeting + RX/TX join ([#51](https://github.com/andymai/stackchan-kai/issues/51)) ([b50beae](https://github.com/andymai/stackchan-kai/commit/b50beae5b02f7be69b3347595a66dff58450053d))
* **firmware:** audio_bench example — playlist of every clip ([#58](https://github.com/andymai/stackchan-kai/issues/58)) ([2d5564f](https://github.com/andymai/stackchan-kai/commit/2d5564f6dc7092b2f63d8852576507ac0f9c6340))
* **firmware:** double-buffer via PSRAM to eliminate direct-draw flicker ([940551c](https://github.com/andymai/stackchan-kai/commit/940551c5767d1221bbc354f1787e2e903dd83758))
* **firmware:** esp-rtos boot + AXP2101 LCD rails ([212dc5c](https://github.com/andymai/stackchan-kai/commit/212dc5c93a3a179bea956ebf1b7f538d3111f1e4))
* **firmware:** ILI9342C via mipidsi — one-shot Avatar render ([9265830](https://github.com/andymai/stackchan-kai/commit/926583005f80c4b4755f196707f7888d36cd5987))
* **firmware:** RMS sample loop — audio task → mouth pipeline live ([#48](https://github.com/andymai/stackchan-kai/issues/48)) ([c1eb250](https://github.com/andymai/stackchan-kai/commit/c1eb250440db2164588045daf951c5a8109f0338))
* **firmware:** time-of-day boot greeting via BM8563 RTC ([#57](https://github.com/andymai/stackchan-kai/issues/57)) ([84e7f15](https://github.com/andymai/stackchan-kai/commit/84e7f1575b15daf6814a6ecc5f2a28084ba998fb))
* FT6336U tap-to-cycle emotion + shared I²C0 bus ([#15](https://github.com/andymai/stackchan-kai/issues/15)) ([b3fb8de](https://github.com/andymai/stackchan-kai/commit/b3fb8de289ac45f9c5537516a79c2dd2e3a4e6cb))
* low-battery hysteresis + USB-power aware override ([#54](https://github.com/andymai/stackchan-kai/issues/54)) ([0741a2e](https://github.com/andymai/stackchan-kai/commit/0741a2edcb9478c34e1fdb4e6739393cedfd2019))
* low-battery sleepy emotion — AXP2101 SoC reader, power task, modifier ([#52](https://github.com/andymai/stackchan-kai/issues/52)) ([d6add63](https://github.com/andymai/stackchan-kai/commit/d6add63fa0a16dc17cb5984c163eab80600f0885))
* pan/tilt servo head motion (+aw9523 extract, pca9685 driver) ([#2](https://github.com/andymai/stackchan-kai/issues/2)) ([9bc40a1](https://github.com/andymai/stackchan-kai/commit/9bc40a11b76108aad8a58af7a941a28fade5ea0c))
* power-button taps + IR NEC RemoteCommand modifier ([#19](https://github.com/andymai/stackchan-kai/issues/19)) ([f29c92e](https://github.com/andymai/stackchan-kai/commit/f29c92e7550fa450be0adb8b94a8b57801e18ddf))
* PY32 WS2812 LED ring + first output-sink path ([#20](https://github.com/andymai/stackchan-kai/issues/20)) ([14dcbd3](https://github.com/andymai/stackchan-kai/commit/14dcbd3be257175f0df973c5cb616c41d92c483b))
* servo position readback + calibration bench binary ([#11](https://github.com/andymai/stackchan-kai/issues/11)) ([890c8f8](https://github.com/andymai/stackchan-kai/commit/890c8f8232f80f3b6f861ded1ec2b2e386fbc31d))
* swap PCA9685 for Feetech SCServo on UART1 (matches real HW) ([#5](https://github.com/andymai/stackchan-kai/issues/5)) ([1ff3376](https://github.com/andymai/stackchan-kai/commit/1ff3376440453924e64cb7497c1e3a8e698fdb48))
* WakeOnVoice modifier — sustained mic activity wakes to Happy ([#55](https://github.com/andymai/stackchan-kai/issues/55)) ([c8729bf](https://github.com/andymai/stackchan-kai/commit/c8729bfbac9d78de54f20c64875d42e8544d0b8c))


### Bug Fixes

* **axp2101:** apply full M5Unified CoreS3 init to stop idle shutdown ([f5bc712](https://github.com/andymai/stackchan-kai/commit/f5bc712073813630f3fe78d1331d918799e55f70))
* **es7210:** drop invented chip-ID check blocking bring-up ([#31](https://github.com/andymai/stackchan-kai/issues/31)) ([304ef58](https://github.com/andymai/stackchan-kai/commit/304ef582e025713f420ab30970781c9a9d11ae64))
* **firmware:** boot on CoreS3 hardware end-to-end ([dba4c89](https://github.com/andymai/stackchan-kai/commit/dba4c89b89ad27b8adc07143a8163607410efd69))
* **firmware:** enable SCServo torque after ping, restore yes-nod gesture ([5398094](https://github.com/andymai/stackchan-kai/commit/5398094e86512d6ff4f928c16471a96f65b0d4e4))
* **firmware:** enable SCServo torque after ping, restore yes-nod gesture ([daf03b7](https://github.com/andymai/stackchan-kai/commit/daf03b771aa6a72235773c4ee7eefc262901ed06))
* **firmware:** I²C 400 kHz, justfile `reattach` recipe + reliability notes ([#34](https://github.com/andymai/stackchan-kai/issues/34)) ([82a462a](https://github.com/andymai/stackchan-kai/commit/82a462a3d50c93207ce60a8b8af4ab12693c6615))
* **firmware:** quiet boot-time warnings, drop SCServo FIFO-overflow spam ([10fd917](https://github.com/andymai/stackchan-kai/commit/10fd917651c66e6c3dcda939654f238e7b0e68ec))
* **firmware:** quiet boot-time warnings, drop SCServo FIFO-overflow spam ([28015fd](https://github.com/andymai/stackchan-kai/commit/28015fdb76c7523c249b4cbff239de33ba692589))
* **firmware:** restore LCD backlight + full AW9523 init on CoreS3 ([31ea98e](https://github.com/andymai/stackchan-kai/commit/31ea98e0d49a9329e72bf35357e227301492e23a))
* **firmware:** retry BMI270 init on I²C timeout, log SCServo angle limits ([0d477e5](https://github.com/andymai/stackchan-kai/commit/0d477e5e2c609e35df8df4279be9083280f56949))
* **firmware:** retry BMI270 init on I²C timeout, log SCServo angle limits ([fdbdbda](https://github.com/andymai/stackchan-kai/commit/fdbdbdaa41c826188fd4b3b37b85ffec9cff2bc1))
* **firmware:** revert I²C to 100 kHz, reduce boot-nod tilt amplitude ([f42315c](https://github.com/andymai/stackchan-kai/commit/f42315cd105f24396f3948c14be1b10e3d6d14f9))
* **firmware:** revert I²C to 100 kHz, reduce boot-nod tilt amplitude ([cb74dca](https://github.com/andymai/stackchan-kai/commit/cb74dcad6caa2c74b7ae1d3434dd9c98f6cd992d))
* **firmware:** satisfy pedantic clippy lints blocking CI ([0a37661](https://github.com/andymai/stackchan-kai/commit/0a37661ab74f4081f3ce5e4ba015236b5bce76c4))
* tilt calibration for offset-encoder unit + asymmetric range support ([#47](https://github.com/andymai/stackchan-kai/issues/47)) ([52b8c4d](https://github.com/andymai/stackchan-kai/commit/52b8c4d47477baf776c82446d02431d08d24f941))

## [0.5.0](https://github.com/andymai/stackchan-kai/compare/v0.4.0...v0.5.0) (2026-04-25)


### Features

* **audio:** codec bring-up + audio signal plumbing (firmware task scaffold) ([#29](https://github.com/andymai/stackchan-kai/issues/29)) ([0ad42aa](https://github.com/andymai/stackchan-kai/commit/0ad42aa851978c008a9c0684445ece99654ee183))
* **audio:** I²S0 master + MCLK, codec bring-up inside task ([#30](https://github.com/andymai/stackchan-kai/issues/30)) ([e080ffd](https://github.com/andymai/stackchan-kai/commit/e080ffd82e3e2200e20736e6c35431bb23420535))
* **audio:** real AW88298 + ES7210 driver impls + control-path benches ([#28](https://github.com/andymai/stackchan-kai/issues/28)) ([bdd9038](https://github.com/andymai/stackchan-kai/commit/bdd9038653198e9df64d315951533ecae5378aae))
* BM8563 wall-clock + LTR-553 AmbientSleepy modifier ([#18](https://github.com/andymai/stackchan-kai/issues/18)) ([8405d0d](https://github.com/andymai/stackchan-kai/commit/8405d0d0f1619d400ff2bd1691995135f9c25316))
* BMI270 IMU + pickup-reaction modifier ([#17](https://github.com/andymai/stackchan-kai/issues/17)) ([8624ceb](https://github.com/andymai/stackchan-kai/commit/8624ceb92bcb355a28fa3c98ee6499171cb12a2b))
* BMM150 magnetometer (9-axis data path) ([#22](https://github.com/andymai/stackchan-kai/issues/22)) ([eea9212](https://github.com/andymai/stackchan-kai/commit/eea921233444d2efe68d7ee502e14be390778c20))
* **core:** MouthOpenAudio modifier + Mouth::mouth_open field ([#32](https://github.com/andymai/stackchan-kai/issues/32)) ([79020ed](https://github.com/andymai/stackchan-kai/commit/79020ed266f510b1bd2da1f7ecc01f8465105737))
* **core:** wire Emotion into a style-field pipeline with eased transitions ([bfd6a3a](https://github.com/andymai/stackchan-kai/commit/bfd6a3a168ad8f6bcece0e5bfc47f01e791ab8ff))
* **dx:** boot PING health check + boot-nod gesture + justfile ([#6](https://github.com/andymai/stackchan-kai/issues/6)) ([3955354](https://github.com/andymai/stackchan-kai/commit/3955354bcbbf904f5dc88f032dccf1327677399f))
* emotion-coupled head motion (EmotionHead modifier) ([#4](https://github.com/andymai/stackchan-kai/issues/4)) ([3f197f1](https://github.com/andymai/stackchan-kai/commit/3f197f106527977da99cdd9ac75dab79462290c4))
* **firmware:** 30 FPS render task with Blink + dirty-check ([46d52ed](https://github.com/andymai/stackchan-kai/commit/46d52ed49ad4024a5d5de8c91b8c2db7c340d326))
* **firmware:** add Breath + IdleDrift to the render stack ([7c1c1af](https://github.com/andymai/stackchan-kai/commit/7c1c1af413a478a2b895c3acc869cb12808516cb))
* **firmware:** double-buffer via PSRAM to eliminate direct-draw flicker ([940551c](https://github.com/andymai/stackchan-kai/commit/940551c5767d1221bbc354f1787e2e903dd83758))
* **firmware:** esp-rtos boot + AXP2101 LCD rails ([212dc5c](https://github.com/andymai/stackchan-kai/commit/212dc5c93a3a179bea956ebf1b7f538d3111f1e4))
* **firmware:** ILI9342C via mipidsi — one-shot Avatar render ([9265830](https://github.com/andymai/stackchan-kai/commit/926583005f80c4b4755f196707f7888d36cd5987))
* FT6336U tap-to-cycle emotion + shared I²C0 bus ([#15](https://github.com/andymai/stackchan-kai/issues/15)) ([b3fb8de](https://github.com/andymai/stackchan-kai/commit/b3fb8de289ac45f9c5537516a79c2dd2e3a4e6cb))
* pan/tilt servo head motion (+aw9523 extract, pca9685 driver) ([#2](https://github.com/andymai/stackchan-kai/issues/2)) ([9bc40a1](https://github.com/andymai/stackchan-kai/commit/9bc40a11b76108aad8a58af7a941a28fade5ea0c))
* power-button taps + IR NEC RemoteCommand modifier ([#19](https://github.com/andymai/stackchan-kai/issues/19)) ([f29c92e](https://github.com/andymai/stackchan-kai/commit/f29c92e7550fa450be0adb8b94a8b57801e18ddf))
* PY32 WS2812 LED ring + first output-sink path ([#20](https://github.com/andymai/stackchan-kai/issues/20)) ([14dcbd3](https://github.com/andymai/stackchan-kai/commit/14dcbd3be257175f0df973c5cb616c41d92c483b))
* servo position readback + calibration bench binary ([#11](https://github.com/andymai/stackchan-kai/issues/11)) ([890c8f8](https://github.com/andymai/stackchan-kai/commit/890c8f8232f80f3b6f861ded1ec2b2e386fbc31d))
* swap PCA9685 for Feetech SCServo on UART1 (matches real HW) ([#5](https://github.com/andymai/stackchan-kai/issues/5)) ([1ff3376](https://github.com/andymai/stackchan-kai/commit/1ff3376440453924e64cb7497c1e3a8e698fdb48))


### Bug Fixes

* **axp2101:** apply full M5Unified CoreS3 init to stop idle shutdown ([f5bc712](https://github.com/andymai/stackchan-kai/commit/f5bc712073813630f3fe78d1331d918799e55f70))
* **es7210:** drop invented chip-ID check blocking bring-up ([#31](https://github.com/andymai/stackchan-kai/issues/31)) ([304ef58](https://github.com/andymai/stackchan-kai/commit/304ef582e025713f420ab30970781c9a9d11ae64))
* **firmware:** boot on CoreS3 hardware end-to-end ([dba4c89](https://github.com/andymai/stackchan-kai/commit/dba4c89b89ad27b8adc07143a8163607410efd69))
* **firmware:** enable SCServo torque after ping, restore yes-nod gesture ([5398094](https://github.com/andymai/stackchan-kai/commit/5398094e86512d6ff4f928c16471a96f65b0d4e4))
* **firmware:** enable SCServo torque after ping, restore yes-nod gesture ([daf03b7](https://github.com/andymai/stackchan-kai/commit/daf03b771aa6a72235773c4ee7eefc262901ed06))
* **firmware:** I²C 400 kHz, justfile `reattach` recipe + reliability notes ([#34](https://github.com/andymai/stackchan-kai/issues/34)) ([82a462a](https://github.com/andymai/stackchan-kai/commit/82a462a3d50c93207ce60a8b8af4ab12693c6615))
* **firmware:** quiet boot-time warnings, drop SCServo FIFO-overflow spam ([10fd917](https://github.com/andymai/stackchan-kai/commit/10fd917651c66e6c3dcda939654f238e7b0e68ec))
* **firmware:** quiet boot-time warnings, drop SCServo FIFO-overflow spam ([28015fd](https://github.com/andymai/stackchan-kai/commit/28015fdb76c7523c249b4cbff239de33ba692589))
* **firmware:** restore LCD backlight + full AW9523 init on CoreS3 ([31ea98e](https://github.com/andymai/stackchan-kai/commit/31ea98e0d49a9329e72bf35357e227301492e23a))
* **firmware:** retry BMI270 init on I²C timeout, log SCServo angle limits ([0d477e5](https://github.com/andymai/stackchan-kai/commit/0d477e5e2c609e35df8df4279be9083280f56949))
* **firmware:** retry BMI270 init on I²C timeout, log SCServo angle limits ([fdbdbda](https://github.com/andymai/stackchan-kai/commit/fdbdbdaa41c826188fd4b3b37b85ffec9cff2bc1))
* **firmware:** revert I²C to 100 kHz, reduce boot-nod tilt amplitude ([f42315c](https://github.com/andymai/stackchan-kai/commit/f42315cd105f24396f3948c14be1b10e3d6d14f9))
* **firmware:** revert I²C to 100 kHz, reduce boot-nod tilt amplitude ([cb74dca](https://github.com/andymai/stackchan-kai/commit/cb74dcad6caa2c74b7ae1d3434dd9c98f6cd992d))
* **firmware:** satisfy pedantic clippy lints blocking CI ([0a37661](https://github.com/andymai/stackchan-kai/commit/0a37661ab74f4081f3ce5e4ba015236b5bce76c4))
* tilt calibration for offset-encoder unit + asymmetric range support ([#47](https://github.com/andymai/stackchan-kai/issues/47)) ([52b8c4d](https://github.com/andymai/stackchan-kai/commit/52b8c4d47477baf776c82446d02431d08d24f941))

## [0.4.0](https://github.com/andymai/stackchan-kai/compare/v0.3.0...v0.4.0) (2026-04-24)


### Features

* **audio:** codec bring-up + audio signal plumbing (firmware task scaffold) ([#29](https://github.com/andymai/stackchan-kai/issues/29)) ([524b9c1](https://github.com/andymai/stackchan-kai/commit/524b9c1f594c5be956384745186369ab6e2f3149))
* **audio:** I²S0 master + MCLK, codec bring-up inside task ([#30](https://github.com/andymai/stackchan-kai/issues/30)) ([dc470ec](https://github.com/andymai/stackchan-kai/commit/dc470ecb5a89a8c0610f6304b3b4f196c5e1c3ae))
* **audio:** real AW88298 + ES7210 driver impls + control-path benches ([#28](https://github.com/andymai/stackchan-kai/issues/28)) ([2d85673](https://github.com/andymai/stackchan-kai/commit/2d8567378feefbcf541c54e7d189e7e13c6f4ebf))
* BM8563 wall-clock + LTR-553 AmbientSleepy modifier ([#18](https://github.com/andymai/stackchan-kai/issues/18)) ([a1f1af8](https://github.com/andymai/stackchan-kai/commit/a1f1af89d0409319cdf8cde60071dd8176ffae3b))
* BMI270 IMU + pickup-reaction modifier ([#17](https://github.com/andymai/stackchan-kai/issues/17)) ([3dae938](https://github.com/andymai/stackchan-kai/commit/3dae938089eaa76b28a5fc258e80a6f44999f4d9))
* BMM150 magnetometer (9-axis data path) ([#22](https://github.com/andymai/stackchan-kai/issues/22)) ([cdd2ff7](https://github.com/andymai/stackchan-kai/commit/cdd2ff79425afbf7f4d5eda89aa6e2c939859444))
* **core:** MouthOpenAudio modifier + Mouth::mouth_open field ([#32](https://github.com/andymai/stackchan-kai/issues/32)) ([8866fe6](https://github.com/andymai/stackchan-kai/commit/8866fe68f2f229ca238926bde28c503fcdf08e24))
* **core:** wire Emotion into a style-field pipeline with eased transitions ([bfd6a3a](https://github.com/andymai/stackchan-kai/commit/bfd6a3a168ad8f6bcece0e5bfc47f01e791ab8ff))
* **dx:** boot PING health check + boot-nod gesture + justfile ([#6](https://github.com/andymai/stackchan-kai/issues/6)) ([e854251](https://github.com/andymai/stackchan-kai/commit/e854251decac986420a04065850fa910dff101d1))
* emotion-coupled head motion (EmotionHead modifier) ([#4](https://github.com/andymai/stackchan-kai/issues/4)) ([f144bb8](https://github.com/andymai/stackchan-kai/commit/f144bb8dcb3f0e810137c0989ac22a0913067eda))
* **firmware:** 30 FPS render task with Blink + dirty-check ([46d52ed](https://github.com/andymai/stackchan-kai/commit/46d52ed49ad4024a5d5de8c91b8c2db7c340d326))
* **firmware:** add Breath + IdleDrift to the render stack ([7c1c1af](https://github.com/andymai/stackchan-kai/commit/7c1c1af413a478a2b895c3acc869cb12808516cb))
* **firmware:** double-buffer via PSRAM to eliminate direct-draw flicker ([940551c](https://github.com/andymai/stackchan-kai/commit/940551c5767d1221bbc354f1787e2e903dd83758))
* **firmware:** esp-rtos boot + AXP2101 LCD rails ([212dc5c](https://github.com/andymai/stackchan-kai/commit/212dc5c93a3a179bea956ebf1b7f538d3111f1e4))
* **firmware:** ILI9342C via mipidsi — one-shot Avatar render ([9265830](https://github.com/andymai/stackchan-kai/commit/926583005f80c4b4755f196707f7888d36cd5987))
* FT6336U tap-to-cycle emotion + shared I²C0 bus ([#15](https://github.com/andymai/stackchan-kai/issues/15)) ([b724304](https://github.com/andymai/stackchan-kai/commit/b7243041f173deaa70d9cdf8b65f3a74430828c3))
* pan/tilt servo head motion (+aw9523 extract, pca9685 driver) ([#2](https://github.com/andymai/stackchan-kai/issues/2)) ([1caa3ce](https://github.com/andymai/stackchan-kai/commit/1caa3ced220093864b65f54dbba34cfe4a6a70c1))
* power-button taps + IR NEC RemoteCommand modifier ([#19](https://github.com/andymai/stackchan-kai/issues/19)) ([0542ced](https://github.com/andymai/stackchan-kai/commit/0542ced96f320938db52c58a436b988f654255f4))
* PY32 WS2812 LED ring + first output-sink path ([#20](https://github.com/andymai/stackchan-kai/issues/20)) ([fe5e598](https://github.com/andymai/stackchan-kai/commit/fe5e5989e6a8a2cee47e324a0ccf4479c336ba75))
* servo position readback + calibration bench binary ([#11](https://github.com/andymai/stackchan-kai/issues/11)) ([e5bf109](https://github.com/andymai/stackchan-kai/commit/e5bf10988ce5bf147b1cf2b5135874196d40255b))
* swap PCA9685 for Feetech SCServo on UART1 (matches real HW) ([#5](https://github.com/andymai/stackchan-kai/issues/5)) ([3d8a14b](https://github.com/andymai/stackchan-kai/commit/3d8a14b371fefd2c5f1803a1ad332c2137cfb4fe))


### Bug Fixes

* **axp2101:** apply full M5Unified CoreS3 init to stop idle shutdown ([f5bc712](https://github.com/andymai/stackchan-kai/commit/f5bc712073813630f3fe78d1331d918799e55f70))
* **es7210:** drop invented chip-ID check blocking bring-up ([#31](https://github.com/andymai/stackchan-kai/issues/31)) ([24f42ae](https://github.com/andymai/stackchan-kai/commit/24f42aeae97f1404df45bfe46c7009352ff657be))
* **firmware:** boot on CoreS3 hardware end-to-end ([dba4c89](https://github.com/andymai/stackchan-kai/commit/dba4c89b89ad27b8adc07143a8163607410efd69))
* **firmware:** I²C 400 kHz, justfile `reattach` recipe + reliability notes ([#34](https://github.com/andymai/stackchan-kai/issues/34)) ([41325ee](https://github.com/andymai/stackchan-kai/commit/41325ee9880ac62ce7331149e92e81fa502b4cf0))
* **firmware:** restore LCD backlight + full AW9523 init on CoreS3 ([31ea98e](https://github.com/andymai/stackchan-kai/commit/31ea98e0d49a9329e72bf35357e227301492e23a))
* **firmware:** satisfy pedantic clippy lints blocking CI ([0a37661](https://github.com/andymai/stackchan-kai/commit/0a37661ab74f4081f3ce5e4ba015236b5bce76c4))

## [0.3.0](https://github.com/andymai/stackchan-kai/compare/v0.2.0...v0.3.0) (2026-04-24)


### Features

* BM8563 wall-clock + LTR-553 AmbientSleepy modifier ([#18](https://github.com/andymai/stackchan-kai/issues/18)) ([a1f1af8](https://github.com/andymai/stackchan-kai/commit/a1f1af89d0409319cdf8cde60071dd8176ffae3b))
* BMI270 IMU + pickup-reaction modifier ([#17](https://github.com/andymai/stackchan-kai/issues/17)) ([3dae938](https://github.com/andymai/stackchan-kai/commit/3dae938089eaa76b28a5fc258e80a6f44999f4d9))
* FT6336U tap-to-cycle emotion + shared I²C0 bus ([#15](https://github.com/andymai/stackchan-kai/issues/15)) ([b724304](https://github.com/andymai/stackchan-kai/commit/b7243041f173deaa70d9cdf8b65f3a74430828c3))
* power-button taps + IR NEC RemoteCommand modifier ([#19](https://github.com/andymai/stackchan-kai/issues/19)) ([0542ced](https://github.com/andymai/stackchan-kai/commit/0542ced96f320938db52c58a436b988f654255f4))

## [0.2.0](https://github.com/andymai/stackchan-kai/compare/v0.1.0...v0.2.0) (2026-04-24)


### Features

* **dx:** boot PING health check + boot-nod gesture + justfile ([#6](https://github.com/andymai/stackchan-kai/issues/6)) ([e854251](https://github.com/andymai/stackchan-kai/commit/e854251decac986420a04065850fa910dff101d1))
* emotion-coupled head motion (EmotionHead modifier) ([#4](https://github.com/andymai/stackchan-kai/issues/4)) ([f144bb8](https://github.com/andymai/stackchan-kai/commit/f144bb8dcb3f0e810137c0989ac22a0913067eda))
* pan/tilt servo head motion (+aw9523 extract, pca9685 driver) ([#2](https://github.com/andymai/stackchan-kai/issues/2)) ([1caa3ce](https://github.com/andymai/stackchan-kai/commit/1caa3ced220093864b65f54dbba34cfe4a6a70c1))
* servo position readback + calibration bench binary ([#11](https://github.com/andymai/stackchan-kai/issues/11)) ([e5bf109](https://github.com/andymai/stackchan-kai/commit/e5bf10988ce5bf147b1cf2b5135874196d40255b))
* swap PCA9685 for Feetech SCServo on UART1 (matches real HW) ([#5](https://github.com/andymai/stackchan-kai/issues/5)) ([3d8a14b](https://github.com/andymai/stackchan-kai/commit/3d8a14b371fefd2c5f1803a1ad332c2137cfb4fe))
