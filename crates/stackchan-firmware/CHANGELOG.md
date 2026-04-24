# Changelog

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
