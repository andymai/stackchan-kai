# Changelog

## [0.100.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.99.0...stackchan-firmware-v0.100.0) (2026-05-20)


### Features

* **mcp:** expose clear_crash + play_dance — last HTTP-twin gap closed ([#517](https://github.com/andymai/stackchan-kai/issues/517)) ([e3d94d4](https://github.com/andymai/stackchan-kai/commit/e3d94d42821f27aba0fd5a68be42e4f8f9006862))

## [0.99.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.98.0...stackchan-firmware-v0.99.0) (2026-05-20)


### Features

* **mcp:** expose mutator tools — set_palette / set_face_target / set_camera_mode / get_head_offsets / set_head_offsets ([#514](https://github.com/andymai/stackchan-kai/issues/514)) ([a54809b](https://github.com/andymai/stackchan-kai/commit/a54809b5b2fac78bc495b045e8342a9805ffcd52))

## [0.98.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.97.3...stackchan-firmware-v0.98.0) (2026-05-20)


### Features

* **mcp:** expose diagnostic tools — get_sensors / get_tasks / get_events / get_crash ([#513](https://github.com/andymai/stackchan-kai/issues/513)) ([893f526](https://github.com/andymai/stackchan-kai/commit/893f52625c6e5fd27fa82b9a7363023238147b3f))

## [0.97.3](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.97.2...stackchan-firmware-v0.97.3) (2026-05-20)

## [0.97.2](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.97.1...stackchan-firmware-v0.97.2) (2026-05-20)


### Bug Fixes

* **firmware:** network + audio hardening pass ([#493](https://github.com/andymai/stackchan-kai/issues/493)) ([7d22f79](https://github.com/andymai/stackchan-kai/commit/7d22f79aeb52457245d7a32ce8524e200cc3fe49))

## [0.97.1](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.97.0...stackchan-firmware-v0.97.1) (2026-05-20)


### Bug Fixes

* **firmware:** toast_info uses ToastLevel::Info, not Warn ([#491](https://github.com/andymai/stackchan-kai/issues/491)) ([9334ec3](https://github.com/andymai/stackchan-kai/commit/9334ec3f24517ca56d253795b1f1bf588da40559))

## [0.97.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.96.0...stackchan-firmware-v0.97.0) (2026-05-19)


### Features

* **mcp:** expose reset / look_at_point / enter_thinking / exit_thinking ([#401](https://github.com/andymai/stackchan-kai/issues/401)) ([8dac71c](https://github.com/andymai/stackchan-kai/commit/8dac71cf15afc8217b103ea691fa11551b143442))

## [0.96.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.95.0...stackchan-firmware-v0.96.0) (2026-05-19)


### Features

* **firmware:** face-level Sad reaction on sidecar failure paths ([#397](https://github.com/andymai/stackchan-kai/issues/397)) ([1e645bb](https://github.com/andymai/stackchan-kai/commit/1e645bbbe24ffe9092dc5526827954adc03b4942))

## [0.95.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.94.0...stackchan-firmware-v0.95.0) (2026-05-18)


### Features

* **core:** face-level Thinking state across the sidecar round-trip ([#394](https://github.com/andymai/stackchan-kai/issues/394)) ([4a3fdfd](https://github.com/andymai/stackchan-kai/commit/4a3fdfddab93f4b3af024725b314072fdff4e577))

## [0.94.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.93.0...stackchan-firmware-v0.94.0) (2026-05-18)


### Features

* **firmware:** folder-push receiver writes /sd/desktop/&lt;char&gt;/&lt;file&gt; ([#384](https://github.com/andymai/stackchan-kai/issues/384)) ([d734622](https://github.com/andymai/stackchan-kai/commit/d734622fcb3a538ab17d8a0f1e9e9e473ab1732a))

## [0.93.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.92.0...stackchan-firmware-v0.93.0) (2026-05-18)


### Features

* **firmware:** persist cmd:name to /sd/DEVICE.NAM + soft-reset ([#383](https://github.com/andymai/stackchan-kai/issues/383)) ([f43977a](https://github.com/andymai/stackchan-kai/commit/f43977abc17df01fac444301808e00fc5e2e658a))

## [0.92.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.91.2...stackchan-firmware-v0.92.0) (2026-05-18)


### Features

* **firmware:** desktop time-sync writes the BM8563 RTC ([#381](https://github.com/andymai/stackchan-kai/issues/381)) ([67f5c12](https://github.com/andymai/stackchan-kai/commit/67f5c1285208e499bab28708f8230b3313736cc4))

## [0.91.2](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.91.1...stackchan-firmware-v0.91.2) (2026-05-18)


### Bug Fixes

* **firmware:** buddy unpair ack honors bond-wipe result + control cleanups ([#378](https://github.com/andymai/stackchan-kai/issues/378)) ([7497ebb](https://github.com/andymai/stackchan-kai/commit/7497ebb8e27227337ca5db7eaf127eaa738662cb))

## [0.91.1](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.91.0...stackchan-firmware-v0.91.1) (2026-05-18)


### Bug Fixes

* **firmware:** buddy_permission — notify on prompt replacement; lift prompt-id cap ([#376](https://github.com/andymai/stackchan-kai/issues/376)) ([28ca829](https://github.com/andymai/stackchan-kai/commit/28ca829ed4ba024a1fb086cc33baba54b2403ba9))

## [0.91.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.90.0...stackchan-firmware-v0.91.0) (2026-05-18)


### Features

* **firmware:** buddy command-surface — status / owner / unpair / turn ([#374](https://github.com/andymai/stackchan-kai/issues/374)) ([dc7140c](https://github.com/andymai/stackchan-kai/commit/dc7140c95249cf6203cf8aea76c3d63ef9fc5e71))

## [0.90.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.89.0...stackchan-firmware-v0.90.0) (2026-05-18)


### Features

* **firmware:** permission decision via back-of-head tap-twice ([#371](https://github.com/andymai/stackchan-kai/issues/371)) ([d7c6684](https://github.com/andymai/stackchan-kai/commit/d7c6684206717afc6bea9371b0e6bc0a54d94514))

## [0.89.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.88.0...stackchan-firmware-v0.89.0) (2026-05-18)


### Features

* **firmware:** buddy_render task + Claude-prefixed BLE name ([#369](https://github.com/andymai/stackchan-kai/issues/369)) ([ac674e1](https://github.com/andymai/stackchan-kai/commit/ac674e1f3a3ba8374363627a5da85da6adf6d450))

## [0.88.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.87.0...stackchan-firmware-v0.88.0) (2026-05-18)


### Features

* **firmware:** NUS GATT service + per-connection buddy line framer ([#368](https://github.com/andymai/stackchan-kai/issues/368)) ([1ff3441](https://github.com/andymai/stackchan-kai/commit/1ff34414ac7ffd7606448f288b8a8600c9fb0ee2))

## [0.87.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.86.1...stackchan-firmware-v0.87.0) (2026-05-17)


### Features

* **firmware:** sidecar bearer auth + per-device session id ([#359](https://github.com/andymai/stackchan-kai/issues/359)) ([7f6ca34](https://github.com/andymai/stackchan-kai/commit/7f6ca34e82fdfda57aa39209c0ab4826239c33db))

## [0.86.1](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.86.0...stackchan-firmware-v0.86.1) (2026-05-15)

## [0.86.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.85.0...stackchan-firmware-v0.86.0) (2026-05-15)


### Features

* **firmware:** operator-tunable wake_word_arena_kib ([#354](https://github.com/andymai/stackchan-kai/issues/354)) ([90fb26d](https://github.com/andymai/stackchan-kai/commit/90fb26db1f8077d81c47f9b628150d9662d6eb99))

## [0.85.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.84.0...stackchan-firmware-v0.85.0) (2026-05-15)


### Features

* **firmware:** operator-tunable wake_word_threshold via BehaviorConfig ([#352](https://github.com/andymai/stackchan-kai/issues/352)) ([e0d5ab0](https://github.com/andymai/stackchan-kai/commit/e0d5ab0ccb782ef579739f1aca046183748a3d4f))

## [0.84.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.83.0...stackchan-firmware-v0.84.0) (2026-05-15)


### Features

* **firmware:** BluFi push ReportWifiStatus on link transition ([#350](https://github.com/andymai/stackchan-kai/issues/350)) ([8256e1c](https://github.com/andymai/stackchan-kai/commit/8256e1ce79885fbba0277d963ba79afcff13bc97))

## [0.83.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.82.1...stackchan-firmware-v0.83.0) (2026-05-15)


### Features

* **firmware:** BluFi status notifications (Arc 3d slice 3) ([#348](https://github.com/andymai/stackchan-kai/issues/348)) ([4c03de1](https://github.com/andymai/stackchan-kai/commit/4c03de17a92c84a2ed9045112953cd3af5e4a735))

## [0.82.1](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.82.0...stackchan-firmware-v0.82.1) (2026-05-15)


### Bug Fixes

* **firmware:** BluFi reject fragmented frames + clarify commit-side comment ([#346](https://github.com/andymai/stackchan-kai/issues/346)) ([0b4b25b](https://github.com/andymai/stackchan-kai/commit/0b4b25b7845f05c309001f4e7d8d463737bdea64))

## [0.82.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.81.0...stackchan-firmware-v0.82.0) (2026-05-15)


### Features

* **firmware:** BluFi SSID/PSK accumulator + ConnectToAp commit ([#344](https://github.com/andymai/stackchan-kai/issues/344)) ([ade81c5](https://github.com/andymai/stackchan-kai/commit/ade81c509c60fa8e63bb4ff3316280562d158468))

## [0.81.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.80.0...stackchan-firmware-v0.81.0) (2026-05-15)


### Features

* **firmware:** BluFi GATT service shell — parse-and-log inbound frames ([#342](https://github.com/andymai/stackchan-kai/issues/342)) ([1ec4d12](https://github.com/andymai/stackchan-kai/commit/1ec4d124dd424317515b6a26732d7b6072abd057))

## [0.80.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.79.0...stackchan-firmware-v0.80.0) (2026-05-15)


### Features

* **firmware:** wake fires through REMOTE_COMMAND_SIGNAL so avatar reacts ([#339](https://github.com/andymai/stackchan-kai/issues/339)) ([f35bb23](https://github.com/andymai/stackchan-kai/commit/f35bb23f97c368da49ea77afc024c62fb93f7524))

## [0.79.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.78.0...stackchan-firmware-v0.79.0) (2026-05-15)


### Features

* **firmware:** on-device wake-word task ([#337](https://github.com/andymai/stackchan-kai/issues/337)) ([7e1adf4](https://github.com/andymai/stackchan-kai/commit/7e1adf4b51bc19164da940b2303dc0743dffa589))

## [0.78.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.77.0...stackchan-firmware-v0.78.0) (2026-05-14)


### Features

* **firmware:** mimic-follower — apply a leader's mDNS pose locally ([#328](https://github.com/andymai/stackchan-kai/issues/328)) ([53fdfaf](https://github.com/andymai/stackchan-kai/commit/53fdfaf3e66de48f3cfc0d5f12d7fc6cb9e48c56))

## [0.77.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.76.0...stackchan-firmware-v0.77.0) (2026-05-14)


### Features

* **firmware:** GET /state/ws — WebSocket avatar push (RFC 6455) ([#326](https://github.com/andymai/stackchan-kai/issues/326)) ([ba76144](https://github.com/andymai/stackchan-kai/commit/ba761447f38c828b920f8cf765c98c8eb53f7e5e))

## [0.76.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.75.0...stackchan-firmware-v0.76.0) (2026-05-14)


### Features

* **firmware:** sidecar agent — push-to-talk capture + HTTP client ([#323](https://github.com/andymai/stackchan-kai/issues/323)) ([f657fb8](https://github.com/andymai/stackchan-kai/commit/f657fb8a14a73016db9ef323246145c781a5981a))

## [0.75.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.74.0...stackchan-firmware-v0.75.0) (2026-05-13)


### Features

* **mcp:** expose push_toast tool + refactor /toast onto parse_toast ([#320](https://github.com/andymai/stackchan-kai/issues/320)) ([2b6da1d](https://github.com/andymai/stackchan-kai/commit/2b6da1d4800b03bc7c189ae2c8a7edb93ec468eb))

## [0.74.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.73.0...stackchan-firmware-v0.74.0) (2026-05-13)


### Features

* **firmware:** UDP audio debug stream ([#318](https://github.com/andymai/stackchan-kai/issues/318)) ([5d67433](https://github.com/andymai/stackchan-kai/commit/5d67433204376a230dd7497c868ca0927dcf47bf))

## [0.73.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.72.0...stackchan-firmware-v0.73.0) (2026-05-13)


### Features

* **firmware:** ES7210 PCM frame ring buffer ([#315](https://github.com/andymai/stackchan-kai/issues/315)) ([0e792ef](https://github.com/andymai/stackchan-kai/commit/0e792ef4d6bf429e0dbe78757c1ddec07ff89361))

## [0.72.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.71.0...stackchan-firmware-v0.72.0) (2026-05-13)


### Features

* **firmware:** persist head offsets to RUNTIME.RON ([#313](https://github.com/andymai/stackchan-kai/issues/313)) ([f6768c8](https://github.com/andymai/stackchan-kai/commit/f6768c8d02b22b115d2937f60534a44830d8ab82))

## [0.71.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.70.0...stackchan-firmware-v0.71.0) (2026-05-13)


### Features

* **core:** named one-shot motions (greet/nod/shake/laugh) ([#308](https://github.com/andymai/stackchan-kai/issues/308)) ([07ae228](https://github.com/andymai/stackchan-kai/commit/07ae228ed432f02d51f3de80d26a19b91126973e))
* **firmware:** auto-torque-release (idle servo power saver) ([#310](https://github.com/andymai/stackchan-kai/issues/310)) ([6bd375d](https://github.com/andymai/stackchan-kai/commit/6bd375dcc9b7feb24d9f79b1eed0696f87339a57))

## [0.70.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.69.0...stackchan-firmware-v0.70.0) (2026-05-13)


### Features

* **firmware:** toast log overlay (opt-in) ([#306](https://github.com/andymai/stackchan-kai/issues/306)) ([bc08775](https://github.com/andymai/stackchan-kai/commit/bc08775859fb4c792a032687c6b0d9e6cae9d01d))

## [0.69.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.68.0...stackchan-firmware-v0.69.0) (2026-05-13)


### Features

* **core:** IdleMicroExpression — random mouth-y nudges at 2–6s ([#305](https://github.com/andymai/stackchan-kai/issues/305)) ([427fa19](https://github.com/andymai/stackchan-kai/commit/427fa19d96b6c2b461fcf7927ab08aa2c5b60b7f))

## [0.68.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.67.1...stackchan-firmware-v0.68.0) (2026-05-13)


### Features

* **core:** on-screen battery overlay (opt-in) ([#304](https://github.com/andymai/stackchan-kai/issues/304)) ([6aa2283](https://github.com/andymai/stackchan-kai/commit/6aa22835ce02313375f46592f3c48ada7b4a77c7))

## [0.67.1](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.67.0...stackchan-firmware-v0.67.1) (2026-05-08)


### Bug Fixes

* **firmware:** WIFI_LINK_WATCH cold-boot race — seed via get() then loop on changed() ([#299](https://github.com/andymai/stackchan-kai/issues/299)) ([2853fd6](https://github.com/andymai/stackchan-kai/commit/2853fd6bd523f12e0ff1eef0ab147b08864fb8f4))

## [0.67.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.66.1...stackchan-firmware-v0.67.0) (2026-05-08)


### Features

* **firmware:** crash recovery — RTC-RAM panic latch + /sd/CRASH.LOG + dashboard banner ([#297](https://github.com/andymai/stackchan-kai/issues/297)) ([f1e29e1](https://github.com/andymai/stackchan-kai/commit/f1e29e19601f740d607eb4fa7dcfde31b50619d8))

## [0.66.1](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.66.0...stackchan-firmware-v0.66.1) (2026-05-08)


### Bug Fixes

* **firmware:** WIFI_LINK_SIGNAL multi-consumer race — replace with Watch ([#294](https://github.com/andymai/stackchan-kai/issues/294)) ([17e03be](https://github.com/andymai/stackchan-kai/commit/17e03be05ca3264b812b25318eb4c69f46c88985))

## [0.66.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.65.0...stackchan-firmware-v0.66.0) (2026-05-08)


### Features

* face geometry presets — POST /face-geometry + selectable silhouettes ([#290](https://github.com/andymai/stackchan-kai/issues/290)) ([cbe2d4f](https://github.com/andymai/stackchan-kai/commit/cbe2d4f1f354ae0d534e1718f5cc0272b1fa1ef0))

## [0.65.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.64.0...stackchan-firmware-v0.65.0) (2026-05-08)


### Features

* dance choreography — POST /dance keyframe stream + DancePlayer modifier ([#288](https://github.com/andymai/stackchan-kai/issues/288)) ([2695ed0](https://github.com/andymai/stackchan-kai/commit/2695ed06de7679d856f3685fc76d184958501951))

## [0.64.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.63.0...stackchan-firmware-v0.64.0) (2026-05-08)


### Features

* **core:** HeadFromBodyGesture — randomized head nudge on swipe / press ([#285](https://github.com/andymai/stackchan-kai/issues/285)) ([1b8f5da](https://github.com/andymai/stackchan-kai/commit/1b8f5daf9c570ad25a8211997c7ca4e5edb2fbb6))
* **firmware:** mDNS pose TXT — yaw/pitch live advertisement ([#284](https://github.com/andymai/stackchan-kai/issues/284)) ([fd3bbec](https://github.com/andymai/stackchan-kai/commit/fd3bbec49abd9c4978284cbf50d69b0593280f56))

## [0.63.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.62.3...stackchan-firmware-v0.63.0) (2026-05-07)


### Features

* **firmware:** operator-commanded sleep mode ([#279](https://github.com/andymai/stackchan-kai/issues/279)) ([93bac33](https://github.com/andymai/stackchan-kai/commit/93bac339bfd2341d56db86d6dc3d53580ec26a50))

## [0.62.3](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.62.2...stackchan-firmware-v0.62.3) (2026-05-07)


### Bug Fixes

* **firmware:** greptile post-merge findings on [#273](https://github.com/andymai/stackchan-kai/issues/273) + [#275](https://github.com/andymai/stackchan-kai/issues/275) ([#277](https://github.com/andymai/stackchan-kai/issues/277)) ([fef55ff](https://github.com/andymai/stackchan-kai/commit/fef55ff75a42e68dc279c39509dc68e6d880e38b))

## [0.62.2](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.62.1...stackchan-firmware-v0.62.2) (2026-05-07)


### Bug Fixes

* round-2 review — disk race in runtime_store + stuck FLASH on OTA fail ([#275](https://github.com/andymai/stackchan-kai/issues/275)) ([e10a31c](https://github.com/andymai/stackchan-kai/commit/e10a31c3a617cdb21834e19eb4da5e1127c9ad70))

## [0.62.1](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.62.0...stackchan-firmware-v0.62.1) (2026-05-07)


### Bug Fixes

* v0.2.0 quality pass — bug fixes + tests + cleanup ([#273](https://github.com/andymai/stackchan-kai/issues/273)) ([b007cbf](https://github.com/andymai/stackchan-kai/commit/b007cbffc7f27db4f525e070c70a6d722a64e53b))

## [0.62.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.61.0...stackchan-firmware-v0.62.0) (2026-05-07)


### Features

* **firmware:** on-device OTA — verify, flash, swap, reboot ([#271](https://github.com/andymai/stackchan-kai/issues/271)) ([b8217ed](https://github.com/andymai/stackchan-kai/commit/b8217eddfaa8d3da7a7633bd6eabfc287526151c))

## [0.61.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.60.0...stackchan-firmware-v0.61.0) (2026-05-07)


### Features

* **net:** MCP take_photo — capture trigger + snapshot URL ([#268](https://github.com/andymai/stackchan-kai/issues/268)) ([0eb1c4b](https://github.com/andymai/stackchan-kai/commit/0eb1c4b907e854a641e0a19124fb940b865cd544))

## [0.60.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.59.0...stackchan-firmware-v0.60.0) (2026-05-07)


### Features

* **firmware:** SD-backed RuntimeStore — persist palette + mood ([#266](https://github.com/andymai/stackchan-kai/issues/266)) ([df1415a](https://github.com/andymai/stackchan-kai/commit/df1415ace028fc1e9f1519f385868833e9e92e7e))

## [0.59.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.58.0...stackchan-firmware-v0.59.0) (2026-05-07)


### Features

* **firmware:** MCP reminder/timer tools — runtime scheduler ([#263](https://github.com/andymai/stackchan-kai/issues/263)) ([91cbf9a](https://github.com/andymai/stackchan-kai/commit/91cbf9a9edd198ae68188ec168f3ab8530914e1f))

## [0.58.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.57.0...stackchan-firmware-v0.58.0) (2026-05-07)


### Features

* **firmware:** hourly chime — config-gated top-of-hour chirp ([#261](https://github.com/andymai/stackchan-kai/issues/261)) ([bf379f7](https://github.com/andymai/stackchan-kai/commit/bf379f7a75bc8b06a2027ad9acd026d1b75ba063))

## [0.57.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.56.0...stackchan-firmware-v0.57.0) (2026-05-07)


### Features

* **net:** ESP-NOW TX path — pose-mirror + heartbeat broadcast ([#255](https://github.com/andymai/stackchan-kai/issues/255)) ([38fb5f4](https://github.com/andymai/stackchan-kai/commit/38fb5f46fde74d724338163e1c57dcd677c3bd86))

## [0.56.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.55.0...stackchan-firmware-v0.56.0) (2026-05-07)


### Features

* **net:** mDNS DNS-SD service records for _stackchan._tcp.local. ([#254](https://github.com/andymai/stackchan-kai/issues/254)) ([5a40567](https://github.com/andymai/stackchan-kai/commit/5a405676bc5c81e22ff61fa8704500cfc9300a02))

## [0.55.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.54.0...stackchan-firmware-v0.55.0) (2026-05-07)


### Features

* **core:** runtime palette swap with named presets ([#252](https://github.com/andymai/stackchan-kai/issues/252)) ([e77ad86](https://github.com/andymai/stackchan-kai/commit/e77ad86f81e411ee97512d15e8a0ccafa7aea439))

## [0.54.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.53.0...stackchan-firmware-v0.54.0) (2026-05-07)


### Features

* **core:** Soliloquy modifier — opt-in autonomous bubble beats ([#250](https://github.com/andymai/stackchan-kai/issues/250)) ([18c108c](https://github.com/andymai/stackchan-kai/commit/18c108c56cd85076c29046383a1f45d0cacc9bb2))

## [0.53.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.52.0...stackchan-firmware-v0.53.0) (2026-05-07)


### Features

* **net:** apply time.tz from STACKCHAN.RON to boot wall-clock ([#248](https://github.com/andymai/stackchan-kai/issues/248)) ([5226ef0](https://github.com/andymai/stackchan-kai/commit/5226ef04f50df71ccf27295584380297ae302fe0))

## [0.52.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.51.0...stackchan-firmware-v0.52.0) (2026-05-07)


### Features

* **net:** POST /face-target endpoint for external CV servers ([#245](https://github.com/andymai/stackchan-kai/issues/245)) ([5679121](https://github.com/andymai/stackchan-kai/commit/567912114c923349654d6b0d4235ac3dad985b0c))

## [0.51.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.50.0...stackchan-firmware-v0.51.0) (2026-05-07)


### Features

* **net:** MCP set_volume + set_mute tools ([#244](https://github.com/andymai/stackchan-kai/issues/244)) ([bf38c8b](https://github.com/andymai/stackchan-kai/commit/bf38c8b09a892117dad555ae14ee61724e787357))

## [0.50.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.49.0...stackchan-firmware-v0.50.0) (2026-05-07)


### Features

* **core:** speech-bubble overlay primitive ([#240](https://github.com/andymai/stackchan-kai/issues/240)) ([fe98f66](https://github.com/andymai/stackchan-kai/commit/fe98f66d8e8185b55a0e8e7e6adf9763cd519c23))

## [0.49.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.48.0...stackchan-firmware-v0.49.0) (2026-05-07)


### Features

* **core:** Angry + Shy decorators with emotion-edge trigger ([#239](https://github.com/andymai/stackchan-kai/issues/239)) ([0f5693a](https://github.com/andymai/stackchan-kai/commit/0f5693a2ae81e70addd2d521dd4506ac71552660))

## [0.48.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.47.0...stackchan-firmware-v0.48.0) (2026-05-07)


### Features

* **firmware:** register LostTargetSearch in render task director ([#238](https://github.com/andymai/stackchan-kai/issues/238)) ([84ecc9e](https://github.com/andymai/stackchan-kai/commit/84ecc9ebcddbc79493b4a1a80c5544640cd7a73b))

## [0.47.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.46.0...stackchan-firmware-v0.47.0) (2026-05-07)


### Features

* **firmware:** ESP-NOW RX task — peer-allowlisted frame ingest ([#233](https://github.com/andymai/stackchan-kai/issues/233)) ([93536b4](https://github.com/andymai/stackchan-kai/commit/93536b496f5fc5c896269ed31493b9b2193dc55a))

## [0.46.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.45.0...stackchan-firmware-v0.46.0) (2026-05-07)


### Features

* **core:** pairing-window scaffolding + Decorator::Pairing ([#231](https://github.com/andymai/stackchan-kai/issues/231)) ([38b14fa](https://github.com/andymai/stackchan-kai/commit/38b14fa1db2bd21bc39e8ebdb337a65d27d116b0))

## [0.45.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.44.0...stackchan-firmware-v0.45.0) (2026-05-07)


### Features

* **core:** listen-window scaffolding + Ear decorator ([#230](https://github.com/andymai/stackchan-kai/issues/230)) ([3e87968](https://github.com/andymai/stackchan-kai/commit/3e8796834dc41b67e706e63c0142a6a1984d62ae))

## [0.44.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.43.0...stackchan-firmware-v0.44.0) (2026-05-07)


### Features

* **net:** minimal MCP server endpoint over JSON-RPC 2.0 ([#227](https://github.com/andymai/stackchan-kai/issues/227)) ([2f11294](https://github.com/andymai/stackchan-kai/commit/2f11294de88743baf1dc0164b207f5d30b41b95a))

## [0.43.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.42.0...stackchan-firmware-v0.43.0) (2026-05-07)


### Features

* **firmware:** GET /camera/snapshot + dashboard view-capture button ([#221](https://github.com/andymai/stackchan-kai/issues/221)) ([ae1ccd0](https://github.com/andymai/stackchan-kai/commit/ae1ccd0068c0f8253f0bd6972d1626710bf262f0))

## [0.42.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.41.0...stackchan-firmware-v0.42.0) (2026-05-07)


### Features

* **core:** mood presets + closed-eye smile on Petted ([#219](https://github.com/andymai/stackchan-kai/issues/219)) ([03c4a51](https://github.com/andymai/stackchan-kai/commit/03c4a51583f7f029de272014dad69faf81a0a984))

## [0.41.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.40.2...stackchan-firmware-v0.41.0) (2026-05-07)


### Features

* **core:** decorator overlay layer + 3 trigger modifiers ([#223](https://github.com/andymai/stackchan-kai/issues/223)) ([057e56e](https://github.com/andymai/stackchan-kai/commit/057e56e5d3ae8b585858fdd0b963a836ae72232b))

## [0.40.2](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.40.1...stackchan-firmware-v0.40.2) (2026-05-07)

## [0.40.1](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.40.0...stackchan-firmware-v0.40.1) (2026-05-05)


### Bug Fixes

* address greptile feedback across the dashboard PRs ([#214](https://github.com/andymai/stackchan-kai/issues/214)) ([2462cf3](https://github.com/andymai/stackchan-kai/commit/2462cf37c498da6f30a5bae2a2842cfb3ca933d6))

## [0.40.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.39.0...stackchan-firmware-v0.40.0) (2026-05-05)


### Features

* **firmware:** recovery & lifecycle — restart + factory-reset ([#212](https://github.com/andymai/stackchan-kai/issues/212)) ([bf99cb4](https://github.com/andymai/stackchan-kai/commit/bf99cb422c6d4df743790dfa9a914df6afdccbef))

## [0.39.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.38.0...stackchan-firmware-v0.39.0) (2026-05-05)


### Features

* **firmware:** settings backup endpoint + reboot_required diff ([#210](https://github.com/andymai/stackchan-kai/issues/210)) ([130101e](https://github.com/andymai/stackchan-kai/commit/130101e3f23c13afe2214e2873306766c325e094))

## [0.38.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.37.0...stackchan-firmware-v0.38.0) (2026-05-05)


### Features

* **firmware:** live introspection — sensors, events, task health ([#207](https://github.com/andymai/stackchan-kai/issues/207)) ([b4741ce](https://github.com/andymai/stackchan-kai/commit/b4741ce1068556789ad15b87ce31b1c3a6dacb5f))

## [0.37.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.36.0...stackchan-firmware-v0.37.0) (2026-05-05)


### Features

* **firmware:** TS + Solid + Vite dashboard pipeline ([#205](https://github.com/andymai/stackchan-kai/issues/205)) ([f9b419a](https://github.com/andymai/stackchan-kai/commit/f9b419a5cf056c7629bdbe07d406dd582b04922c))

## [0.36.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.35.0...stackchan-firmware-v0.36.0) (2026-05-01)


### Features

* **firmware:** voice-gated frame-centre face cascade ([#194](https://github.com/andymai/stackchan-kai/issues/194)) ([ae13d18](https://github.com/andymai/stackchan-kai/commit/ae13d1888e7b97a21d9a55a93f3e84a4d5f641e3))

## [0.35.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.34.1...stackchan-firmware-v0.35.0) (2026-05-01)


### Features

* **net:** operator-tunable tracker FOV via PUT /settings ([#192](https://github.com/andymai/stackchan-kai/issues/192)) ([ce51d58](https://github.com/andymai/stackchan-kai/commit/ce51d588e62769cb2c613af51eb47214454a2754))

## [0.34.1](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.34.0...stackchan-firmware-v0.34.1) (2026-04-30)

## [0.34.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.33.0...stackchan-firmware-v0.34.0) (2026-04-30)


### Features

* **firmware:** camera capture-to-SD via HTTP + BLE triggers ([#188](https://github.com/andymai/stackchan-kai/issues/188)) ([fd84d79](https://github.com/andymai/stackchan-kai/commit/fd84d79a77691d9fde164b5a8f7fbb98a2c5701c))

## [0.33.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.32.0...stackchan-firmware-v0.33.0) (2026-04-30)


### Features

* **firmware:** camera-mode toggle via HTTP + BLE control planes ([#186](https://github.com/andymai/stackchan-kai/issues/186)) ([f2dcfae](https://github.com/andymai/stackchan-kai/commit/f2dcfae6b8fd1c168af7411b7bfb9a059247fb97))

## [0.32.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.31.0...stackchan-firmware-v0.32.0) (2026-04-30)


### Features

* **firmware:** BLE control-plane writes for audio + avatar ([#179](https://github.com/andymai/stackchan-kai/issues/179)) ([a290eb0](https://github.com/andymai/stackchan-kai/commit/a290eb0cd75a505844b9263c6cabaaaae8e84515))

## [0.31.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.30.0...stackchan-firmware-v0.31.0) (2026-04-28)


### Features

* **firmware:** BLE pairing + bonding + LCD passkey overlay ([#174](https://github.com/andymai/stackchan-kai/issues/174)) ([88cb303](https://github.com/andymai/stackchan-kai/commit/88cb30300067ca18508956c11dcbc8df801aa1b6))

## [0.30.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.29.0...stackchan-firmware-v0.30.0) (2026-04-28)


### Features

* **firmware:** BLE Wi-Fi provisioning + soft reconnect ([#176](https://github.com/andymai/stackchan-kai/issues/176)) ([b1f348f](https://github.com/andymai/stackchan-kai/commit/b1f348f1b4f04dc7f9f011ca2badd47d6c852e51))

## [0.29.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.28.0...stackchan-firmware-v0.29.0) (2026-04-28)


### Features

* **firmware:** BLE peripheral with read-only GATT ([#172](https://github.com/andymai/stackchan-kai/issues/172)) ([865813b](https://github.com/andymai/stackchan-kai/commit/865813b2b174b18aa5b257d5f31aa1af35402dc3))

## [0.28.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.27.0...stackchan-firmware-v0.28.0) (2026-04-28)


### Features

* **firmware:** POST /volume + POST /mute (persisted) ([#170](https://github.com/andymai/stackchan-kai/issues/170)) ([f27d2e1](https://github.com/andymai/stackchan-kai/commit/f27d2e1e047371a382386562a14df76540945b09))

## [0.27.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.26.0...stackchan-firmware-v0.27.0) (2026-04-27)


### Features

* **firmware:** preserve-current sentinel for PSK + token on PUT /settings ([#165](https://github.com/andymai/stackchan-kai/issues/165)) ([0356b95](https://github.com/andymai/stackchan-kai/commit/0356b95afda6b159e5511fae87b3b74afdfc1e3b))

## [0.26.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.25.0...stackchan-firmware-v0.26.0) (2026-04-27)


### Features

* **firmware:** POST /speak + audio queue eviction policy ([#163](https://github.com/andymai/stackchan-kai/issues/163)) ([9024675](https://github.com/andymai/stackchan-kai/commit/902467580c17c69190c6c148231bc057ce677c73))

## [0.25.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.24.0...stackchan-firmware-v0.25.0) (2026-04-27)


### Features

* **firmware:** bearer-token auth on PUT/POST routes ([#160](https://github.com/andymai/stackchan-kai/issues/160)) ([625538a](https://github.com/andymai/stackchan-kai/commit/625538a14e97cc7fd371efd4fa48af11f83133a0))
* **firmware:** operator dashboard at GET / ([#157](https://github.com/andymai/stackchan-kai/issues/157)) ([0470c23](https://github.com/andymai/stackchan-kai/commit/0470c23ac2879620d1a3ade74452d140f2209038))
* **firmware:** SSE GET /state/stream + concurrent HTTP workers ([#156](https://github.com/andymai/stackchan-kai/issues/156)) ([0c531ff](https://github.com/andymai/stackchan-kai/commit/0c531fff2a04bf055be0d3aa51c26e62a8fe6b69))

## [0.24.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.23.0...stackchan-firmware-v0.24.0) (2026-04-27)


### Features

* **firmware:** HTTP GET/PUT /settings (atomic SD writeback) ([#154](https://github.com/andymai/stackchan-kai/issues/154)) ([5622096](https://github.com/andymai/stackchan-kai/commit/5622096a58fc9b0c775e935080907f5a8c39f89c))

## [0.23.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.22.0...stackchan-firmware-v0.23.0) (2026-04-27)


### Features

* **firmware:** HTTP POST control plane (emotion, look-at, reset) ([#152](https://github.com/andymai/stackchan-kai/issues/152)) ([f2f9076](https://github.com/andymai/stackchan-kai/commit/f2f9076edb846f90381b75fbb991632f7331d8a0))

## [0.22.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.21.0...stackchan-firmware-v0.22.0) (2026-04-27)


### Features

* **firmware:** minimal mDNS hostname responder ([#150](https://github.com/andymai/stackchan-kai/issues/150)) ([3fd1af1](https://github.com/andymai/stackchan-kai/commit/3fd1af13bc3f462f4e9adc87f3efefe6c2cf1172))

## [0.21.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.20.0...stackchan-firmware-v0.21.0) (2026-04-27)


### Features

* **firmware:** minimal HTTP control plane (GET /health, GET /state) ([#148](https://github.com/andymai/stackchan-kai/issues/148)) ([997f99c](https://github.com/andymai/stackchan-kai/commit/997f99c4ff42baea63d057fa72cc5fcb01c24ca8))

## [0.20.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.19.0...stackchan-firmware-v0.20.0) (2026-04-27)


### Features

* **firmware:** SNTP-on-link-up writes BM8563 RTC ([#146](https://github.com/andymai/stackchan-kai/issues/146)) ([6ea7094](https://github.com/andymai/stackchan-kai/commit/6ea70941c5c23698fd7ca8c2db587a6e3ff6e8fa))

## [0.19.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.18.0...stackchan-firmware-v0.19.0) (2026-04-27)


### Features

* **firmware:** esp-radio Wi-Fi station with offline-first retry ([#144](https://github.com/andymai/stackchan-kai/issues/144)) ([4638833](https://github.com/andymai/stackchan-kai/commit/463883365b50d31b7b3ee0808f136b881d0d4960))

## [0.18.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.17.2...stackchan-firmware-v0.18.0) (2026-04-26)


### Features

* **firmware:** SD-card boot config read with offline-first fallback ([#142](https://github.com/andymai/stackchan-kai/issues/142)) ([4081b72](https://github.com/andymai/stackchan-kai/commit/4081b72bcd67c2068f53a13d5288b1ad13a0b42b))

## [0.17.2](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.17.1...stackchan-firmware-v0.17.2) (2026-04-26)


### Bug Fixes

* **firmware:** time-bound log_angle_limits read so headless boot doesn't hang ([#140](https://github.com/andymai/stackchan-kai/issues/140)) ([c74f4aa](https://github.com/andymai/stackchan-kai/commit/c74f4aa14e7f464fdef4721e459e706d7b4e9e48))

## [0.17.1](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.17.0...stackchan-firmware-v0.17.1) (2026-04-26)


### Bug Fixes

* **firmware:** drop unused stackchan-net dep until SD I/O lands ([#138](https://github.com/andymai/stackchan-kai/issues/138)) ([8bd2d92](https://github.com/andymai/stackchan-kai/commit/8bd2d9234dedb414d1c79504c6f59052decd7c7b))

## [0.17.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.16.0...stackchan-firmware-v0.17.0) (2026-04-26)


### Features

* **net:** bare RON parser + GPIO35 OE-flip SD-SPI adapter ([#136](https://github.com/andymai/stackchan-kai/issues/136)) ([d8d399e](https://github.com/andymai/stackchan-kai/commit/d8d399e827e2963d7563cd5fc427e4649ebed9e4))

## [0.16.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.15.0...stackchan-firmware-v0.16.0) (2026-04-26)


### Features

* speech synthesis framework with baked backend ([#132](https://github.com/andymai/stackchan-kai/issues/132)) ([8c9018f](https://github.com/andymai/stackchan-kai/commit/8c9018f709f80ea4e24d0215ca8e57d7c3a5dd8c))

## [0.15.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.14.0...stackchan-firmware-v0.15.0) (2026-04-26)


### Features

* **core:** dormant mode quiets head servos when nothing's happening ([#128](https://github.com/andymai/stackchan-kai/issues/128)) ([1d60ae9](https://github.com/andymai/stackchan-kai/commit/1d60ae902ed1cdb0d26129ede92105912e0aa70d))

## [0.14.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.13.1...stackchan-firmware-v0.14.0) (2026-04-26)


### Features

* **firmware:** tracking-trace cargo feature for camera-pipeline observability ([#126](https://github.com/andymai/stackchan-kai/issues/126)) ([5797ccd](https://github.com/andymai/stackchan-kai/commit/5797ccd86077406021adb198541122ca1c60e2ec))

## [0.13.1](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.13.0...stackchan-firmware-v0.13.1) (2026-04-26)


### Bug Fixes

* **firmware:** bump CASCADE_PERIOD 4→8 to relieve cooperative-scheduler starvation ([#123](https://github.com/andymai/stackchan-kai/issues/123)) ([beed888](https://github.com/andymai/stackchan-kai/commit/beed888ad36cc6a10074d7c10504d659f859d510))

## [0.13.0](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.12.1...stackchan-firmware-v0.13.0) (2026-04-26)


### Features

* face tracking with engagement-driven gaze and lost-target search ([#121](https://github.com/andymai/stackchan-kai/issues/121)) ([142fc1c](https://github.com/andymai/stackchan-kai/commit/142fc1c9e2bdc161a2d5c36492f0fec85a3dcbfe))

## [0.12.1](https://github.com/andymai/stackchan-kai/compare/stackchan-firmware-v0.12.0...stackchan-firmware-v0.12.1) (2026-04-26)

## [0.12.0](https://github.com/andymai/stackchan-kai/compare/v0.11.0...v0.12.0) (2026-04-26)


### ⚠ BREAKING CHANGES

* engine architecture — Entity component model + Director registry + Skill surface

### Features

* **audio:** codec bring-up + audio signal plumbing (firmware task scaffold) ([#29](https://github.com/andymai/stackchan-kai/issues/29)) ([0ad42aa](https://github.com/andymai/stackchan-kai/commit/0ad42aa851978c008a9c0684445ece99654ee183))
* **audio:** I²S0 master + MCLK, codec bring-up inside task ([#30](https://github.com/andymai/stackchan-kai/issues/30)) ([e080ffd](https://github.com/andymai/stackchan-kai/commit/e080ffd82e3e2200e20736e6c35431bb23420535))
* **audio:** real AW88298 + ES7210 driver impls + control-path benches ([#28](https://github.com/andymai/stackchan-kai/issues/28)) ([bdd9038](https://github.com/andymai/stackchan-kai/commit/bdd9038653198e9df64d315951533ecae5378aae))
* block-grid motion tracker crate + bench example ([#63](https://github.com/andymai/stackchan-kai/issues/63)) ([9865af5](https://github.com/andymai/stackchan-kai/commit/9865af5f17143f7df73d9e88e91ef2aa8c44ef55))
* BM8563 wall-clock + LTR-553 AmbientSleepy modifier ([#18](https://github.com/andymai/stackchan-kai/issues/18)) ([8405d0d](https://github.com/andymai/stackchan-kai/commit/8405d0d0f1619d400ff2bd1691995135f9c25316))
* BMI270 IMU + pickup-reaction modifier ([#17](https://github.com/andymai/stackchan-kai/issues/17)) ([8624ceb](https://github.com/andymai/stackchan-kai/commit/8624ceb92bcb355a28fa3c98ee6499171cb12a2b))
* BMM150 magnetometer (9-axis data path) ([#22](https://github.com/andymai/stackchan-kai/issues/22)) ([eea9212](https://github.com/andymai/stackchan-kai/commit/eea921233444d2efe68d7ee502e14be390778c20))
* camera-preview mode — GC0308 + LCD_CAM ping-pong DMA, long-press toggle ([#60](https://github.com/andymai/stackchan-kai/issues/60)) ([5cb0b62](https://github.com/andymai/stackchan-kai/commit/5cb0b626823f5b06c8efafc25f2e55f4930dd915))
* **core:** AttentionFromTracking — Cognition modifier + Attention::Tracking (3/5) ([af335a4](https://github.com/andymai/stackchan-kai/commit/af335a4bfe2c15f591849c18f3f14259bf8eec5e))
* **core:** AttentionFromTracking — Cognition-phase modifier + Attention::Tracking variant ([96211dd](https://github.com/andymai/stackchan-kai/commit/96211ddf50e5d206acfa2977c6cf0d370e4c2d74))
* **core:** BodyGesture modifier — Press/Swipe/Release on Si12T strip ([e3c709e](https://github.com/andymai/stackchan-kai/commit/e3c709ebd507bb43a886626e9b8fc0d746a1fa4d))
* **core:** BodyGesture modifier — Press/Swipe/Release on Si12T strip ([23c778c](https://github.com/andymai/stackchan-kai/commit/23c778c0ba3f8d353d46c99090aa4d939a094f58))
* **core:** Handling skill — IMU → mind.intent (PickedUp/Shaken/Tilted) ([02f8d79](https://github.com/andymai/stackchan-kai/commit/02f8d7923669dc99a8db9b0a1377ccff62db8015))
* **core:** Handling skill — IMU → mind.intent (PickedUp/Shaken/Tilted) ([3603d9b](https://github.com/andymai/stackchan-kai/commit/3603d9b93e0243d2d9acba30d6a31af7f03045e9))
* **core:** head + eye reactions to Attention::Tracking ([910a965](https://github.com/andymai/stackchan-kai/commit/910a965ffe71efa7e1d5d2e3ccb04f1e908815ce))
* **core:** head + eye reactions to Attention::Tracking (4/5) ([9e87a57](https://github.com/andymai/stackchan-kai/commit/9e87a5753449876f38cd2f16c731251b1fd0d656))
* **core:** IntentStyle modifier — visible reaction to mind.intent ([90095ec](https://github.com/andymai/stackchan-kai/commit/90095eccdef6caf1ffe2eb69e22f357a44a74b36))
* **core:** IntentStyle modifier — visible reaction to mind.intent ([2a5fc58](https://github.com/andymai/stackchan-kai/commit/2a5fc58557cf0f4331bdec83bfd9ec380ab9bd4b))
* **core:** LookAtSound skill + ListenHead motion modifier ([fcd47ee](https://github.com/andymai/stackchan-kai/commit/fcd47ee788d9c46511343b82a03e9742c843e0d1))
* **core:** LookAtSound skill + ListenHead motion modifier ([9b1f7e3](https://github.com/andymai/stackchan-kai/commit/9b1f7e3a33bd63ba2a2d6db2a6fa5cd7e6b5b03b))
* **core:** MouthOpenAudio modifier + Mouth::mouth_open field ([#32](https://github.com/andymai/stackchan-kai/issues/32)) ([79020ed](https://github.com/andymai/stackchan-kai/commit/79020ed266f510b1bd2da1f7ecc01f8465105737))
* **core:** perception.tracking field + firmware drain ([99939bc](https://github.com/andymai/stackchan-kai/commit/99939bc8bae33df1ee9ebb5f5f219d2bae5cb0de))
* **core:** perception.tracking field + firmware drain (2/5) ([4057c06](https://github.com/andymai/stackchan-kai/commit/4057c06a94f7a90e1b72f4772d8946a40f501b6d))
* **core:** Petting skill — sustained body-touch → mind.intent=BeingPet ([82225df](https://github.com/andymai/stackchan-kai/commit/82225df369879f946b4ecf53895fca144a393aff))
* **core:** Petting skill — sustained body-touch → mind.intent=BeingPet ([2ca154c](https://github.com/andymai/stackchan-kai/commit/2ca154cf6388ee461aeb01ce07786854916eb59d))
* **core:** StartleOnLoud modifier — RX RMS rising edge → mind.intent=HearingLoud ([9d4ffec](https://github.com/andymai/stackchan-kai/commit/9d4ffec8e7082b7ce7fbf186ebb3f7804608f96d))
* **core:** StartleOnLoud modifier — sound-reactive startle chain ([07d2bcd](https://github.com/andymai/stackchan-kai/commit/07d2bcd4ab5bb4be3089e3be9905ee5a15329ffc))
* **core:** tracking realism — multi-target detection + microsaccades + eye-leads-head + engagement-aware blink/breath ([#115](https://github.com/andymai/stackchan-kai/issues/115)) ([e667e26](https://github.com/andymai/stackchan-kai/commit/e667e263c351791eb35686910b984485ade47871))
* **core:** wire Emotion into a style-field pipeline with eased transitions ([bfd6a3a](https://github.com/andymai/stackchan-kai/commit/bfd6a3a168ad8f6bcece0e5bfc47f01e791ab8ff))
* **dx:** boot PING health check + boot-nod gesture + justfile ([#6](https://github.com/andymai/stackchan-kai/issues/6)) ([3955354](https://github.com/andymai/stackchan-kai/commit/3955354bcbbf904f5dc88f032dccf1327677399f))
* emotion-coupled head motion (EmotionHead modifier) ([#4](https://github.com/andymai/stackchan-kai/issues/4)) ([3f197f1](https://github.com/andymai/stackchan-kai/commit/3f197f106527977da99cdd9ac75dab79462290c4))
* emotion-transition chirps — pickup, wake, low-battery audio cues ([#56](https://github.com/andymai/stackchan-kai/issues/56)) ([f097f8c](https://github.com/andymai/stackchan-kai/commit/f097f8c78f0e8299f088e8aac8180a17b89ad623))
* **firmware:** 30 FPS render task with Blink + dirty-check ([46d52ed](https://github.com/andymai/stackchan-kai/commit/46d52ed49ad4024a5d5de8c91b8c2db7c340d326))
* **firmware:** add Breath + IdleDrift to the render stack ([7c1c1af](https://github.com/andymai/stackchan-kai/commit/7c1c1af413a478a2b895c3acc869cb12808516cb))
* **firmware:** always-on camera capture + tracker wired into engine signal ([264a352](https://github.com/andymai/stackchan-kai/commit/264a352b35d45d8ad01da97a81c57f986ad5edc8))
* **firmware:** always-on camera capture + tracker wired into engine signal (1/5) ([4ce57ef](https://github.com/andymai/stackchan-kai/commit/4ce57efac08e7cd79e3df17eadef3ca0347386e1))
* **firmware:** audio TX clip queue + low-battery alert beep ([#53](https://github.com/andymai/stackchan-kai/issues/53)) ([c0ddc3f](https://github.com/andymai/stackchan-kai/commit/c0ddc3fdbc55bfe68f4f26b345a49f34cc158246))
* **firmware:** audio TX path — speaker bring-up + boot greeting + RX/TX join ([#51](https://github.com/andymai/stackchan-kai/issues/51)) ([b50beae](https://github.com/andymai/stackchan-kai/commit/b50beae5b02f7be69b3347595a66dff58450053d))
* **firmware:** audio_bench example — playlist of every clip ([#58](https://github.com/andymai/stackchan-kai/issues/58)) ([2d5564f](https://github.com/andymai/stackchan-kai/commit/2d5564f6dc7092b2f63d8852576507ac0f9c6340))
* **firmware:** double-buffer via PSRAM to eliminate direct-draw flicker ([940551c](https://github.com/andymai/stackchan-kai/commit/940551c5767d1221bbc354f1787e2e903dd83758))
* **firmware:** embassy task watchdog supervisor ([#85](https://github.com/andymai/stackchan-kai/issues/85)) ([57dc280](https://github.com/andymai/stackchan-kai/commit/57dc2800bd40d7d01d7b28135d2a1474cf89abf1))
* **firmware:** esp-rtos boot + AXP2101 LCD rails ([212dc5c](https://github.com/andymai/stackchan-kai/commit/212dc5c93a3a179bea956ebf1b7f538d3111f1e4))
* **firmware:** I²C bus probe bench ([9a39872](https://github.com/andymai/stackchan-kai/commit/9a39872dc32a06d1002099359d50d4454db108f8))
* **firmware:** I²C bus probe bench ([49e4074](https://github.com/andymai/stackchan-kai/commit/49e40743fa6a6e622bed4bb63be42a554eae4b2a))
* **firmware:** i2c_probe also reads register 0x00 for chip-ID disambiguation ([fcf34f1](https://github.com/andymai/stackchan-kai/commit/fcf34f17577a6730faa010f09fa0961a0a753262))
* **firmware:** i2c_probe also reads register 0x00 for chip-ID disambiguation ([63231c2](https://github.com/andymai/stackchan-kai/commit/63231c2d4f40fa0120d6b5c4a6c26c66969dc5d5))
* **firmware:** ILI9342C via mipidsi — one-shot Avatar render ([9265830](https://github.com/andymai/stackchan-kai/commit/926583005f80c4b4755f196707f7888d36cd5987))
* **firmware:** RMS sample loop — audio task → mouth pipeline live ([#48](https://github.com/andymai/stackchan-kai/issues/48)) ([c1eb250](https://github.com/andymai/stackchan-kai/commit/c1eb250440db2164588045daf951c5a8109f0338))
* **firmware:** time-of-day boot greeting via BM8563 RTC ([#57](https://github.com/andymai/stackchan-kai/issues/57)) ([84e7f15](https://github.com/andymai/stackchan-kai/commit/84e7f1575b15daf6814a6ecc5f2a28084ba998fb))
* **firmware:** wire Si12T body-touch into engine perception ([8d8a50d](https://github.com/andymai/stackchan-kai/commit/8d8a50dff858c0b6234e2a7afe4c399b0c4b597c))
* **firmware:** wire Si12T body-touch into the engine perception ([fc7f808](https://github.com/andymai/stackchan-kai/commit/fc7f808213a110b76c02407a1099cb8394a6992f))
* FT6336U tap-to-cycle emotion + shared I²C0 bus ([#15](https://github.com/andymai/stackchan-kai/issues/15)) ([b3fb8de](https://github.com/andymai/stackchan-kai/commit/b3fb8de289ac45f9c5537516a79c2dd2e3a4e6cb))
* low-battery hysteresis + USB-power aware override ([#54](https://github.com/andymai/stackchan-kai/issues/54)) ([0741a2e](https://github.com/andymai/stackchan-kai/commit/0741a2edcb9478c34e1fdb4e6739393cedfd2019))
* low-battery sleepy emotion — AXP2101 SoC reader, power task, modifier ([#52](https://github.com/andymai/stackchan-kai/issues/52)) ([d6add63](https://github.com/andymai/stackchan-kai/commit/d6add63fa0a16dc17cb5984c163eab80600f0885))
* pan/tilt servo head motion (+aw9523 extract, pca9685 driver) ([#2](https://github.com/andymai/stackchan-kai/issues/2)) ([9bc40a1](https://github.com/andymai/stackchan-kai/commit/9bc40a11b76108aad8a58af7a941a28fade5ea0c))
* power-button taps + IR NEC RemoteCommand modifier ([#19](https://github.com/andymai/stackchan-kai/issues/19)) ([f29c92e](https://github.com/andymai/stackchan-kai/commit/f29c92e7550fa450be0adb8b94a8b57801e18ddf))
* PY32 WS2812 LED ring + first output-sink path ([#20](https://github.com/andymai/stackchan-kai/issues/20)) ([14dcbd3](https://github.com/andymai/stackchan-kai/commit/14dcbd3be257175f0df973c5cb616c41d92c483b))
* servo position readback + calibration bench binary ([#11](https://github.com/andymai/stackchan-kai/issues/11)) ([890c8f8](https://github.com/andymai/stackchan-kai/commit/890c8f8232f80f3b6f861ded1ec2b2e386fbc31d))
* **si12t:** real driver implementation + bench ([af9fd60](https://github.com/andymai/stackchan-kai/commit/af9fd6029fd68f0df2a1ca7067336fb75adf1af8))
* **si12t:** real driver implementation + bench ([f25c36f](https://github.com/andymai/stackchan-kai/commit/f25c36f4b92f7e81ac0afdf1c4007e2a11eaca57))
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


### Code Refactoring

* engine architecture — Entity component model + Director registry + Skill surface ([f16c7e9](https://github.com/andymai/stackchan-kai/commit/f16c7e97c86be20fe151f41d0366a6edb6ed89ee))

## [0.11.0](https://github.com/andymai/stackchan-kai/compare/v0.9.7...v0.10.0) (2026-04-26)


### Features

* **firmware:** I²C bus probe bench ([#97](https://github.com/andymai/stackchan-kai/issues/97))
* **firmware:** wire Si12T body-touch into engine perception ([#99](https://github.com/andymai/stackchan-kai/issues/99))
* **firmware:** i2c_probe also reads register 0x00 for chip-ID disambiguation ([#104](https://github.com/andymai/stackchan-kai/issues/104))
* **firmware:** drop time-of-day boot greeting + tx-active gating for sound-reactive modifiers ([#106](https://github.com/andymai/stackchan-kai/issues/106))
* **firmware:** always-on camera capture + tracker wired into engine signal ([#109](https://github.com/andymai/stackchan-kai/issues/109))


### Refactors

* apply naming convention sweep across the inventory ([#108](https://github.com/andymai/stackchan-kai/issues/108))

## [0.10.0](https://github.com/andymai/stackchan-kai/compare/v0.9.7...v0.10.0) (2026-04-25)


### Features

* **firmware:** embassy task watchdog supervisor ([#85](https://github.com/andymai/stackchan-kai/issues/85)) ([57dc280](https://github.com/andymai/stackchan-kai/commit/57dc2800bd40d7d01d7b28135d2a1474cf89abf1))

## [0.9.7](https://github.com/andymai/stackchan-kai/compare/v0.9.6...v0.9.7) (2026-04-25)

## [0.9.6](https://github.com/andymai/stackchan-kai/compare/v0.9.5...v0.9.6) (2026-04-25)

## [0.9.5](https://github.com/andymai/stackchan-kai/compare/v0.9.4...v0.9.5) (2026-04-25)

## [0.9.4](https://github.com/andymai/stackchan-kai/compare/v0.9.3...v0.9.4) (2026-04-25)

## [0.9.3](https://github.com/andymai/stackchan-kai/compare/v0.9.2...v0.9.3) (2026-04-25)

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
