# stackchan-kai development tasks.
#
# Install just: https://github.com/casey/just
# Install espup + esp toolchain:
#   cargo install espup
#   espup install
#   source $HOME/export-esp.sh
#
# PORT defaults to /dev/ttyACM1 (Andy's CoreS3 USB-Serial-JTAG). Override
# with `just PORT=/dev/ttyACM0 flash` if your device enumerates differently.

set shell := ["bash", "-cu"]

# Path to the release firmware ELF. Used by `flash`, `monitor`, `fmr`.
firmware_elf := "target/xtensa-esp32s3-none-elf/release/stackchan-firmware"

# Default port for CoreS3 USB-Serial-JTAG.
# Linux enumerates as /dev/ttyACM*, macOS as /dev/cu.usbmodem*.
# Override by prefixing `just PORT=/dev/cu.usbmodem2101 …`.
PORT := if os() == "macos" { "/dev/cu.usbmodem2101" } else { "/dev/ttyACM1" }

# On Linux the `dialout` group gate requires `sg dialout -c '…'`; macOS
# grants serial access directly, so the wrapper is a no-op passthrough.
_serial_prefix := if os() == "macos" { "" } else { "sg dialout -c '" }
_serial_suffix := if os() == "macos" { "" } else { "'" }

# Default: list available recipes.
default:
    @just --list

# ----- Session-start sanity check ------------------------------------------

# Print toolchain availability, device enumeration, working-tree state,
# and recent CI status. Read-only; no side effects. Useful at session
# start (human or AI) before reaching for code.
dev:
    @bash scripts/dev-status.sh

# ----- Host-side -----------------------------------------------------------

# Fast inner-loop check — fmt + cargo check, no clippy or tests.
# Sub-second feedback for "did my edit compile?" iteration. Use this
# while iterating on a single file; run `just check` before committing.
check-fast:
    cargo fmt --check
    cargo check --workspace --exclude stackchan-firmware --all-features --all-targets --quiet

# Fast host checks — the same gates the pre-commit hook runs.
check:
    cargo fmt --check
    cargo clippy --workspace --exclude stackchan-firmware --all-features --all-targets -- -D warnings
    cargo test --workspace --exclude stackchan-firmware --all-features

# Doc-drift guard — flag crates whose src/ has diverged > N commits from
# their README.md. Warn-only by default. Pass `STRICT=1` for non-zero
# exit on drift (CI uses this).
doc-drift:
    @bash scripts/check-doc-drift.sh {{ if env_var_or_default("STRICT", "") == "1" { "--strict" } else { "" } }}

# Boot-log regression guard — diff /tmp/scfmr.log (last `just fmr` /
# `just fmr-agent` flash) against `tests/golden/boot.txt`. Reports
# missing golden lines (regression) and unexpected WARN/ERROR lines
# (new noise) with known-noise filtered. Run after every flash to catch
# silent regressions. Pass `--update` to rewrite the golden from the
# current live log when boot output changes intentionally.
verify-boot *args:
    @bash scripts/check-boot.sh {{args}}

# Everything the CI host job runs (adds doc-lint + cargo-deny).
ci: check
    cargo deny check
    RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --workspace --exclude stackchan-firmware --all-features

# MSRV check — matches the `msrv` CI job. Requires `rustup toolchain install 1.88`.
msrv:
    cargo +1.88 build --workspace --exclude stackchan-firmware --all-features

# ----- Firmware (requires `source ~/export-esp.sh` first) ------------------

# Firmware-side compile check. Runs from inside the firmware crate so the
# per-crate `.cargo/config.toml` (target + `build-std`) actually applies —
# `-p stackchan-firmware` from workspace root silently misses that.
check-firmware:
    cd crates/stackchan-firmware && cargo +esp check

# Firmware strict clippy (matches the CI firmware job).
clippy-firmware:
    cd crates/stackchan-firmware && cargo +esp clippy --release -- -D warnings

# Full release build of the firmware binary.
build-firmware:
    cd crates/stackchan-firmware && cargo +esp build --release

# ----- Flash + monitor ----------------------------------------------------
#
# These recipes go through espflash over the serial-JTAG port. On Linux
# (distrobox), the `sg dialout` wrapper is injected automatically via
# `_serial_prefix`/`_serial_suffix`; on macOS the commands run directly.
#
# ## USB-Serial-JTAG reliability
#
# Prefer `fmr` (combined flash + monitor) over separate `flash; monitor`
# calls — each espflash invocation toggles DTR/RTS to reset the chip,
# and back-to-back resets against ESP32-S3's USB-Serial-JTAG peripheral
# can wedge the USB enumeration until a physical power cycle. The
# combined form issues one reset and transitions straight to monitor,
# keeping the port open. See `just reattach` for a no-reset way to
# pick up a running device's log without reflashing.

# Flash the latest release build. Rebuilds first.
# Prefer `fmr` for normal flash-and-monitor cycles — this recipe is
# split out only for CI or scripted workflows that don't want a monitor
# attached.
flash: build-firmware
    {{_serial_prefix}}espflash flash --port {{PORT}} {{firmware_elf}}{{_serial_suffix}}

# Monitor defmt logs from a running device (no reflash). Exits on Ctrl+C.
# Default form triggers a chip reset on attach — use `just reattach`
# instead to preserve the current boot state.
monitor:
    {{_serial_prefix}}espflash monitor --port {{PORT}} --log-format defmt --elf {{firmware_elf}}{{_serial_suffix}}

# Re-attach to a running device *without* resetting it. Useful when a
# monitor session dropped (`Ctrl+C`, terminal closed, ssh dropped) and
# you want to pick up the log stream without restarting the firmware.
# Also the safer choice when debugging the USB-JTAG disconnect pattern.
reattach:
    {{_serial_prefix}}espflash monitor --no-reset --port {{PORT}} --log-format defmt --elf {{firmware_elf}}{{_serial_suffix}}

# Flash + monitor in one recipe. `fmr` = flash-monitor-reload, the
# default inner-loop verb. Build first, then flash, then stream logs.
# One port-open, one reset — preferred over split `flash; monitor`.
fmr: build-firmware
    {{_serial_prefix}}espflash flash --monitor --log-format defmt --port {{PORT}} {{firmware_elf}}{{_serial_suffix}}

# Agent-friendly fmr: kicks `just fmr` off inside a tmux session, tees
# the output to /tmp/scfmr.log, and polls until "boot complete" appears
# (with a 90 s timeout). Returns 0 on clean boot, 2 on panic, 3 on
# timeout. Designed for AI agents and other non-TTY shells where
# espflash's interactive monitor would otherwise fail to start. See
# CLAUDE.md "Flashing from an agent / non-TTY shell".
fmr-agent:
    @bash scripts/fmr-agent.sh

# Path prefix for release bench example ELFs.
example_elf_dir := "target/xtensa-esp32s3-none-elf/release/examples"

# Calibration bench: flashes the sweep-and-print example + streams its
# defmt output. The bench binary halts after one full sweep; re-flash
# the main firmware with `just flash` or `just fmr` when done.
bench:
    cd crates/stackchan-firmware && cargo +esp build --release --example bench
    {{_serial_prefix}}espflash flash --monitor --log-format defmt --port {{PORT}} {{example_elf_dir}}/bench{{_serial_suffix}}

# Magnetometer bench: streams trim-compensated BMM150 readings at 5 Hz.
# Look for total field magnitude `sqrt(|B|²)` in the 25-65 µT range
# (earth field); deviations are hard-iron offsets from the nearby
# SCServo motors. Re-flash the main firmware with `just fmr` when done.
mag-bench:
    cd crates/stackchan-firmware && cargo +esp build --release --example mag_bench
    {{_serial_prefix}}espflash flash --monitor --log-format defmt --port {{PORT}} {{example_elf_dir}}/mag_bench{{_serial_suffix}}

# LED-ring bench: cycles through each Emotion palette entry every 2 s,
# independent of the modifier pipeline. Useful for verifying the PY32
# WS2812 fan-out without the main render stack in the loop.
leds-bench:
    cd crates/stackchan-firmware && cargo +esp build --release --example leds_bench
    {{_serial_prefix}}espflash flash --monitor --log-format defmt --port {{PORT}} {{example_elf_dir}}/leds_bench{{_serial_suffix}}

# AW88298 control-path bench: runs the amp's full I²C init sequence
# (reset → enable → configure I2S 16 kHz mono → mute → disable boost)
# and logs a heartbeat. Does NOT stream audio — I2S wiring lands in the
# follow-up audio-task PR. Verifies chip presence and register-sequence
# acceptance only.
aw88298-bench:
    cd crates/stackchan-firmware && cargo +esp build --release --example aw88298_bench
    {{_serial_prefix}}espflash flash --monitor --log-format defmt --port {{PORT}} {{example_elf_dir}}/aw88298_bench{{_serial_suffix}}

# ES7210 control-path bench: runs the ADC's full I²C init sequence
# (reset → clock tree for 12.288 MHz / 16 kHz → mic1+2 power-on → latch
# reset) and logs a heartbeat. Does NOT capture audio — I2S wiring
# lands in the follow-up audio-task PR. Verifies chip presence and
# register-sequence acceptance only.
es7210-bench:
    cd crates/stackchan-firmware && cargo +esp build --release --example es7210_bench
    {{_serial_prefix}}espflash flash --monitor --log-format defmt --port {{PORT}} {{example_elf_dir}}/es7210_bench{{_serial_suffix}}

# Audio playlist bench: brings up the full audio stack (I²S + AW88298
# + ES7210) and loops through every clip in the chirp library
# (BOOT_GREETING, time-of-day variants, WAKE_CHIRP, pickup chirp,
# low-battery alert) with 800 ms gaps. Use this when tuning clip
# amplitudes / durations / pitches without rebuilding the full
# firmware.
audio-bench:
    cd crates/stackchan-firmware && cargo +esp build --release --example audio_bench
    {{_serial_prefix}}espflash flash --monitor --log-format defmt --port {{PORT}} {{example_elf_dir}}/audio_bench{{_serial_suffix}}

# Tilt extremes calibration: drives the pitch servo through 0° → ±50°
# in 5° steps (5° past `MAX_TILT_DEG`'s safety bound) and reads back
# the live encoder, stopping the sweep early once readings plateau.
# Use the SUMMARY/SUGGEST lines to set EEPROM angle limits and
# `TILT_TRIM_DEG` in `head.rs`. Re-flash main firmware with
# `just fmr` when done.
tilt-extremes:
    cd crates/stackchan-firmware && cargo +esp build --release --example tilt_extremes
    {{_serial_prefix}}espflash flash --monitor --log-format defmt --port {{PORT}} {{example_elf_dir}}/tilt_extremes{{_serial_suffix}}

# Tilt freewheel diagnostic: disables torque on the pitch servo and
# live-streams the encoder reading at 5 Hz so the operator can hand-
# rotate the head and verify whether the internal position sensor is
# tracking. Use after `tilt-extremes` flags STUCK + OVERLOAD to
# distinguish "encoder dead" from "controller stuck in OVERLOAD".
tilt-freewheel:
    cd crates/stackchan-firmware && cargo +esp build --release --example tilt_freewheel
    {{_serial_prefix}}espflash flash --monitor --log-format defmt --port {{PORT}} {{example_elf_dir}}/tilt_freewheel{{_serial_suffix}}

# Ambient light sensor bench: streams LTR-553 lux readings at 2 Hz +
# proximity counts. Verifies init sequence + driver presence.
ambient-bench:
    cd crates/stackchan-firmware && cargo +esp build --release --example ambient_bench
    {{_serial_prefix}}espflash flash --monitor --log-format defmt --port {{PORT}} {{example_elf_dir}}/ambient_bench{{_serial_suffix}}

# IR-NEC bench: decodes incoming NEC frames from the IR receiver on
# GPIO21 (RMT channel 7) and prints `(address, command)` tuples. Use to
# discover your remote's codes for the `RemoteCommand` modifier mapping.
ir-bench:
    cd crates/stackchan-firmware && cargo +esp build --release --example ir_bench
    {{_serial_prefix}}espflash flash --monitor --log-format defmt --port {{PORT}} {{example_elf_dir}}/ir_bench{{_serial_suffix}}

# FT6336U touch bench: streams raw touch events (point count, x, y,
# pressure) so calibration drift or wiring issues surface as obviously-
# wrong coordinates.
touch-bench:
    cd crates/stackchan-firmware && cargo +esp build --release --example touch_bench
    {{_serial_prefix}}espflash flash --monitor --log-format defmt --port {{PORT}} {{example_elf_dir}}/touch_bench{{_serial_suffix}}

# BMI270 IMU bench: streams accel + gyro at 100 Hz with magnitude
# computed. Use to characterize gravity vector + bias on a static unit
# before hand-tuning PickupReaction thresholds.
imu-bench:
    cd crates/stackchan-firmware && cargo +esp build --release --example imu_bench
    {{_serial_prefix}}espflash flash --monitor --log-format defmt --port {{PORT}} {{example_elf_dir}}/imu_bench{{_serial_suffix}}

# Block-grid motion tracker bench: brings up the camera path and runs
# the `tracker` crate over each frame, logging motion centroids and
# proposed pan/tilt deltas. No servos commanded — algorithm validation
# only.
tracker-bench:
    cd crates/stackchan-firmware && cargo +esp build --release --example tracker_bench
    {{_serial_prefix}}espflash flash --monitor --log-format defmt --port {{PORT}} {{example_elf_dir}}/tracker_bench{{_serial_suffix}}
