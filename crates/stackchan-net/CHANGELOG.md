# Changelog

## [0.10.0](https://github.com/andymai/stackchan-kai/compare/stackchan-net-v0.9.0...stackchan-net-v0.10.0) (2026-05-20)


### Features

* **mcp:** get_sensor_history — rolling 60s sensor window for LLM grounding ([#527](https://github.com/andymai/stackchan-kai/issues/527)) ([48e6395](https://github.com/andymai/stackchan-kai/commit/48e63950e4b28cc8486f9c11418fff57466b4db7))

## [0.9.0](https://github.com/andymai/stackchan-kai/compare/stackchan-net-v0.8.1...stackchan-net-v0.9.0) (2026-05-20)


### Features

* **mcp:** schedule_motion — schedule a canonical motion to fire in N seconds ([#525](https://github.com/andymai/stackchan-kai/issues/525)) ([59e22c4](https://github.com/andymai/stackchan-kai/commit/59e22c426b3b1fe21e416ff4d3bb41d680d6a56d))

## [0.8.1](https://github.com/andymai/stackchan-kai/compare/stackchan-net-v0.8.0...stackchan-net-v0.8.1) (2026-05-20)


### Bug Fixes

* **net:** requires_reboot covers all behavior fields captured at task spawn ([#523](https://github.com/andymai/stackchan-kai/issues/523)) ([4a6d83d](https://github.com/andymai/stackchan-kai/commit/4a6d83d22e2dcfce1a4621db129b0518bb0a279f))

## [0.8.0](https://github.com/andymai/stackchan-kai/compare/stackchan-net-v0.7.0...stackchan-net-v0.8.0) (2026-05-20)


### Features

* **mcp:** expose set_behavior_flag — single-field BehaviorConfig mutate ([#521](https://github.com/andymai/stackchan-kai/issues/521)) ([1767aeb](https://github.com/andymai/stackchan-kai/commit/1767aebc27c19cc209635fc2ff24b42485116ccc))

## [0.7.0](https://github.com/andymai/stackchan-kai/compare/stackchan-net-v0.6.0...stackchan-net-v0.7.0) (2026-05-20)


### Features

* **mcp:** expose get_health — MCP twin of GET /health ([#519](https://github.com/andymai/stackchan-kai/issues/519)) ([f8aeb96](https://github.com/andymai/stackchan-kai/commit/f8aeb96712398e7b4ed825907d328ae3ea69f4c0))

## [0.6.0](https://github.com/andymai/stackchan-kai/compare/stackchan-net-v0.5.0...stackchan-net-v0.6.0) (2026-05-20)


### Features

* **mcp:** expose clear_crash + play_dance — last HTTP-twin gap closed ([#517](https://github.com/andymai/stackchan-kai/issues/517)) ([e3d94d4](https://github.com/andymai/stackchan-kai/commit/e3d94d42821f27aba0fd5a68be42e4f8f9006862))

## [0.5.0](https://github.com/andymai/stackchan-kai/compare/stackchan-net-v0.4.0...stackchan-net-v0.5.0) (2026-05-20)


### Features

* **mcp:** expose mutator tools — set_palette / set_face_target / set_camera_mode / get_head_offsets / set_head_offsets ([#514](https://github.com/andymai/stackchan-kai/issues/514)) ([a54809b](https://github.com/andymai/stackchan-kai/commit/a54809b5b2fac78bc495b045e8342a9805ffcd52))

## [0.4.0](https://github.com/andymai/stackchan-kai/compare/stackchan-net-v0.3.0...stackchan-net-v0.4.0) (2026-05-20)


### Features

* **mcp:** expose diagnostic tools — get_sensors / get_tasks / get_events / get_crash ([#513](https://github.com/andymai/stackchan-kai/issues/513)) ([893f526](https://github.com/andymai/stackchan-kai/commit/893f52625c6e5fd27fa82b9a7363023238147b3f))

## [0.3.0](https://github.com/andymai/stackchan-kai/compare/stackchan-net-v0.2.22...stackchan-net-v0.3.0) (2026-05-20)


### Features

* **blufi:** frame parser + builder + CRC16 foundation ([#330](https://github.com/andymai/stackchan-kai/issues/330)) ([a877903](https://github.com/andymai/stackchan-kai/commit/a877903efa3c7b7fccbf9062dd19461d8bbcae49))
* **core:** 3D IK lookAtPoint — Attention::Point + /look-at-point HTTP route ([#286](https://github.com/andymai/stackchan-kai/issues/286)) ([9eb9d51](https://github.com/andymai/stackchan-kai/commit/9eb9d51135dad62807a5a7a24386b13f6c4c03a4))
* **core:** expand emotion palette with 7 new variants ([#216](https://github.com/andymai/stackchan-kai/issues/216)) ([21f6b20](https://github.com/andymai/stackchan-kai/commit/21f6b20dd497f82f8f9efbda134b87869e5f69f4))
* **core:** listen-window scaffolding + Ear decorator ([#230](https://github.com/andymai/stackchan-kai/issues/230)) ([3e87968](https://github.com/andymai/stackchan-kai/commit/3e8796834dc41b67e706e63c0142a6a1984d62ae))
* **core:** mood presets + closed-eye smile on Petted ([#219](https://github.com/andymai/stackchan-kai/issues/219)) ([03c4a51](https://github.com/andymai/stackchan-kai/commit/03c4a51583f7f029de272014dad69faf81a0a984))
* **core:** named one-shot motions (greet/nod/shake/laugh) ([#308](https://github.com/andymai/stackchan-kai/issues/308)) ([07ae228](https://github.com/andymai/stackchan-kai/commit/07ae228ed432f02d51f3de80d26a19b91126973e))
* **core:** on-screen battery overlay (opt-in) ([#304](https://github.com/andymai/stackchan-kai/issues/304)) ([6aa2283](https://github.com/andymai/stackchan-kai/commit/6aa22835ce02313375f46592f3c48ada7b4a77c7))
* **core:** pairing-window scaffolding + Decorator::Pairing ([#231](https://github.com/andymai/stackchan-kai/issues/231)) ([38b14fa](https://github.com/andymai/stackchan-kai/commit/38b14fa1db2bd21bc39e8ebdb337a65d27d116b0))
* **core:** runtime palette swap with named presets ([#252](https://github.com/andymai/stackchan-kai/issues/252)) ([e77ad86](https://github.com/andymai/stackchan-kai/commit/e77ad86f81e411ee97512d15e8a0ccafa7aea439))
* **core:** Soliloquy modifier — opt-in autonomous bubble beats ([#250](https://github.com/andymai/stackchan-kai/issues/250)) ([18c108c](https://github.com/andymai/stackchan-kai/commit/18c108c56cd85076c29046383a1f45d0cacc9bb2))
* dance choreography — POST /dance keyframe stream + DancePlayer modifier ([#288](https://github.com/andymai/stackchan-kai/issues/288)) ([2695ed0](https://github.com/andymai/stackchan-kai/commit/2695ed06de7679d856f3685fc76d184958501951))
* face geometry presets — POST /face-geometry + selectable silhouettes ([#290](https://github.com/andymai/stackchan-kai/issues/290)) ([cbe2d4f](https://github.com/andymai/stackchan-kai/commit/cbe2d4f1f354ae0d534e1718f5cc0272b1fa1ef0))
* **firmware:** auto-torque-release (idle servo power saver) ([#310](https://github.com/andymai/stackchan-kai/issues/310)) ([6bd375d](https://github.com/andymai/stackchan-kai/commit/6bd375dcc9b7feb24d9f79b1eed0696f87339a57))
* **firmware:** bearer-token auth on PUT/POST routes ([#160](https://github.com/andymai/stackchan-kai/issues/160)) ([625538a](https://github.com/andymai/stackchan-kai/commit/625538a14e97cc7fd371efd4fa48af11f83133a0))
* **firmware:** BLE control-plane writes for audio + avatar ([#179](https://github.com/andymai/stackchan-kai/issues/179)) ([a290eb0](https://github.com/andymai/stackchan-kai/commit/a290eb0cd75a505844b9263c6cabaaaae8e84515))
* **firmware:** BluFi GATT service shell — parse-and-log inbound frames ([#342](https://github.com/andymai/stackchan-kai/issues/342)) ([1ec4d12](https://github.com/andymai/stackchan-kai/commit/1ec4d124dd424317515b6a26732d7b6072abd057))
* **firmware:** BluFi status notifications (Arc 3d slice 3) ([#348](https://github.com/andymai/stackchan-kai/issues/348)) ([4c03de1](https://github.com/andymai/stackchan-kai/commit/4c03de17a92c84a2ed9045112953cd3af5e4a735))
* **firmware:** camera capture-to-SD via HTTP + BLE triggers ([#188](https://github.com/andymai/stackchan-kai/issues/188)) ([fd84d79](https://github.com/andymai/stackchan-kai/commit/fd84d79a77691d9fde164b5a8f7fbb98a2c5701c))
* **firmware:** camera-mode toggle via HTTP + BLE control planes ([#186](https://github.com/andymai/stackchan-kai/issues/186)) ([f2dcfae](https://github.com/andymai/stackchan-kai/commit/f2dcfae6b8fd1c168af7411b7bfb9a059247fb97))
* **firmware:** crash recovery — RTC-RAM panic latch + /sd/CRASH.LOG + dashboard banner ([#297](https://github.com/andymai/stackchan-kai/issues/297)) ([f1e29e1](https://github.com/andymai/stackchan-kai/commit/f1e29e19601f740d607eb4fa7dcfde31b50619d8))
* **firmware:** ESP-NOW RX task — peer-allowlisted frame ingest ([#233](https://github.com/andymai/stackchan-kai/issues/233)) ([93536b4](https://github.com/andymai/stackchan-kai/commit/93536b496f5fc5c896269ed31493b9b2193dc55a))
* **firmware:** hourly chime — config-gated top-of-hour chirp ([#261](https://github.com/andymai/stackchan-kai/issues/261)) ([bf379f7](https://github.com/andymai/stackchan-kai/commit/bf379f7a75bc8b06a2027ad9acd026d1b75ba063))
* **firmware:** HTTP GET/PUT /settings (atomic SD writeback) ([#154](https://github.com/andymai/stackchan-kai/issues/154)) ([5622096](https://github.com/andymai/stackchan-kai/commit/5622096a58fc9b0c775e935080907f5a8c39f89c))
* **firmware:** MCP reminder/timer tools — runtime scheduler ([#263](https://github.com/andymai/stackchan-kai/issues/263)) ([91cbf9a](https://github.com/andymai/stackchan-kai/commit/91cbf9a9edd198ae68188ec168f3ab8530914e1f))
* **firmware:** mDNS pose TXT — yaw/pitch live advertisement ([#284](https://github.com/andymai/stackchan-kai/issues/284)) ([fd3bbec](https://github.com/andymai/stackchan-kai/commit/fd3bbec49abd9c4978284cbf50d69b0593280f56))
* **firmware:** mimic-follower — apply a leader's mDNS pose locally ([#328](https://github.com/andymai/stackchan-kai/issues/328)) ([53fdfaf](https://github.com/andymai/stackchan-kai/commit/53fdfaf3e66de48f3cfc0d5f12d7fc6cb9e48c56))
* **firmware:** on-device wake-word task ([#337](https://github.com/andymai/stackchan-kai/issues/337)) ([7e1adf4](https://github.com/andymai/stackchan-kai/commit/7e1adf4b51bc19164da940b2303dc0743dffa589))
* **firmware:** operator-commanded sleep mode ([#279](https://github.com/andymai/stackchan-kai/issues/279)) ([93bac33](https://github.com/andymai/stackchan-kai/commit/93bac339bfd2341d56db86d6dc3d53580ec26a50))
* **firmware:** operator-tunable wake_word_arena_kib ([#354](https://github.com/andymai/stackchan-kai/issues/354)) ([90fb26d](https://github.com/andymai/stackchan-kai/commit/90fb26db1f8077d81c47f9b628150d9662d6eb99))
* **firmware:** operator-tunable wake_word_threshold via BehaviorConfig ([#352](https://github.com/andymai/stackchan-kai/issues/352)) ([e0d5ab0](https://github.com/andymai/stackchan-kai/commit/e0d5ab0ccb782ef579739f1aca046183748a3d4f))
* **firmware:** POST /volume + POST /mute (persisted) ([#170](https://github.com/andymai/stackchan-kai/issues/170)) ([f27d2e1](https://github.com/andymai/stackchan-kai/commit/f27d2e1e047371a382386562a14df76540945b09))
* **firmware:** preserve-current sentinel for PSK + token on PUT /settings ([#165](https://github.com/andymai/stackchan-kai/issues/165)) ([0356b95](https://github.com/andymai/stackchan-kai/commit/0356b95afda6b159e5511fae87b3b74afdfc1e3b))
* **firmware:** runtime servo offset calibration via HTTP ([#258](https://github.com/andymai/stackchan-kai/issues/258)) ([b298e60](https://github.com/andymai/stackchan-kai/commit/b298e606150dfc96af47df804f8df5f5225eddf2))
* **firmware:** SD-card boot config read with offline-first fallback ([#142](https://github.com/andymai/stackchan-kai/issues/142)) ([4081b72](https://github.com/andymai/stackchan-kai/commit/4081b72bcd67c2068f53a13d5288b1ad13a0b42b))
* **firmware:** sidecar agent — push-to-talk capture + HTTP client ([#323](https://github.com/andymai/stackchan-kai/issues/323)) ([f657fb8](https://github.com/andymai/stackchan-kai/commit/f657fb8a14a73016db9ef323246145c781a5981a))
* **firmware:** sidecar bearer auth + per-device session id ([#359](https://github.com/andymai/stackchan-kai/issues/359)) ([7f6ca34](https://github.com/andymai/stackchan-kai/commit/7f6ca34e82fdfda57aa39209c0ab4826239c33db))
* **firmware:** toast log overlay (opt-in) ([#306](https://github.com/andymai/stackchan-kai/issues/306)) ([bc08775](https://github.com/andymai/stackchan-kai/commit/bc08775859fb4c792a032687c6b0d9e6cae9d01d))
* **firmware:** UDP audio debug stream ([#318](https://github.com/andymai/stackchan-kai/issues/318)) ([5d67433](https://github.com/andymai/stackchan-kai/commit/5d67433204376a230dd7497c868ca0927dcf47bf))
* **mcp:** expose push_toast tool + refactor /toast onto parse_toast ([#320](https://github.com/andymai/stackchan-kai/issues/320)) ([2b6da1d](https://github.com/andymai/stackchan-kai/commit/2b6da1d4800b03bc7c189ae2c8a7edb93ec468eb))
* **mcp:** expose reset / look_at_point / enter_thinking / exit_thinking ([#401](https://github.com/andymai/stackchan-kai/issues/401)) ([8dac71c](https://github.com/andymai/stackchan-kai/commit/8dac71cf15afc8217b103ea691fa11551b143442))
* **net:** apply time.tz from STACKCHAN.RON to boot wall-clock ([#248](https://github.com/andymai/stackchan-kai/issues/248)) ([5226ef0](https://github.com/andymai/stackchan-kai/commit/5226ef04f50df71ccf27295584380297ae302fe0))
* **net:** bare RON parser + GPIO35 OE-flip SD-SPI adapter ([#136](https://github.com/andymai/stackchan-kai/issues/136)) ([d8d399e](https://github.com/andymai/stackchan-kai/commit/d8d399e827e2963d7563cd5fc427e4649ebed9e4))
* **net:** ed25519 OTA signature verification — ship the verifier ([#270](https://github.com/andymai/stackchan-kai/issues/270)) ([2084c35](https://github.com/andymai/stackchan-kai/commit/2084c35d462315e750cbbe15cab8684d81407f24))
* **net:** ESP-NOW TX path — pose-mirror + heartbeat broadcast ([#255](https://github.com/andymai/stackchan-kai/issues/255)) ([38fb5f4](https://github.com/andymai/stackchan-kai/commit/38fb5f46fde74d724338163e1c57dcd677c3bd86))
* **net:** EspNowConfig block in STACKCHAN.RON schema ([#232](https://github.com/andymai/stackchan-kai/issues/232)) ([d3f7587](https://github.com/andymai/stackchan-kai/commit/d3f7587795b24cef2d2da46da99aae9e35b95f6d))
* **net:** MCP set_volume + set_mute tools ([#244](https://github.com/andymai/stackchan-kai/issues/244)) ([bf38c8b](https://github.com/andymai/stackchan-kai/commit/bf38c8b09a892117dad555ae14ee61724e787357))
* **net:** MCP take_photo — capture trigger + snapshot URL ([#268](https://github.com/andymai/stackchan-kai/issues/268)) ([0eb1c4b](https://github.com/andymai/stackchan-kai/commit/0eb1c4b907e854a641e0a19124fb940b865cd544))
* **net:** minimal MCP server endpoint over JSON-RPC 2.0 ([#227](https://github.com/andymai/stackchan-kai/issues/227)) ([2f11294](https://github.com/andymai/stackchan-kai/commit/2f11294de88743baf1dc0164b207f5d30b41b95a))
* **net:** operator-tunable tracker FOV via PUT /settings ([#192](https://github.com/andymai/stackchan-kai/issues/192)) ([ce51d58](https://github.com/andymai/stackchan-kai/commit/ce51d588e62769cb2c613af51eb47214454a2754))
* **net:** OTA image format + signed-image parser ([#228](https://github.com/andymai/stackchan-kai/issues/228)) ([85191cb](https://github.com/andymai/stackchan-kai/commit/85191cb3333e0b4667692571dee77253adb22633))
* **net:** POST /face-target endpoint for external CV servers ([#245](https://github.com/andymai/stackchan-kai/issues/245)) ([5679121](https://github.com/andymai/stackchan-kai/commit/567912114c923349654d6b0d4235ac3dad985b0c))
* **net:** scaffold stackchan-net crate with RON config schema v1 ([#134](https://github.com/andymai/stackchan-kai/issues/134)) ([e13ea78](https://github.com/andymai/stackchan-kai/commit/e13ea78851bdb1462b384abd7e85119c3dfed6a0))


### Bug Fixes

* **firmware:** toast_info uses ToastLevel::Info, not Warn ([#491](https://github.com/andymai/stackchan-kai/issues/491)) ([9334ec3](https://github.com/andymai/stackchan-kai/commit/9334ec3f24517ca56d253795b1f1bf588da40559))
* **net:** bare.rs duplicate-key rejection (parity with bare_json.rs) ([#489](https://github.com/andymai/stackchan-kai/issues/489)) ([42c6fa4](https://github.com/andymai/stackchan-kai/commit/42c6fa41c8851ae362f0cfe1cd97b2a73f011586))
* **net:** don't redact empty wifi.psk on render ([#406](https://github.com/andymai/stackchan-kai/issues/406)) ([1ec84d5](https://github.com/andymai/stackchan-kai/commit/1ec84d5c4a6521609895cc9f7e771e9b7dd746e2))
* **net:** input-validation tightening across stackchan-net ([#492](https://github.com/andymai/stackchan-kai/issues/492)) ([f9a77e7](https://github.com/andymai/stackchan-kai/commit/f9a77e7816fa05407f19c5e981967edbaae4eeb0))
* **net:** reject redaction sentinel on disk-loaded configs ([#490](https://github.com/andymai/stackchan-kai/issues/490)) ([03ef649](https://github.com/andymai/stackchan-kai/commit/03ef64935ddb8dd414df16766088ff9397b8a5c6))
* v0.2.0 quality pass — bug fixes + tests + cleanup ([#273](https://github.com/andymai/stackchan-kai/issues/273)) ([b007cbf](https://github.com/andymai/stackchan-kai/commit/b007cbffc7f27db4f525e070c70a6d722a64e53b))
