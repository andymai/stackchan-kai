# stackchan-rs development tasks.
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

# Default port for CoreS3 USB-Serial-JTAG. Override by prefixing `just PORT=/dev/ttyACM0 …`.
PORT := "/dev/ttyACM1"

# Default: list available recipes.
default:
    @just --list

# ----- Host-side -----------------------------------------------------------

# Fast host checks — the same gates the pre-commit hook runs.
check:
    cargo fmt --check
    cargo clippy --workspace --exclude stackchan-firmware --all-features --all-targets -- -D warnings
    cargo test --workspace --exclude stackchan-firmware --all-features

# Everything the CI host job runs (adds doc-lint + cargo-deny).
ci: check
    cargo deny check
    RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --workspace --exclude stackchan-firmware --all-features

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

# ----- Flash + monitor (requires `sg dialout` on this distrobox) -----------
#
# probe-rs is blocked by SELinux on Andy's Aurora-distrobox setup, so these
# recipes go through espflash over the serial-JTAG port. The `sg dialout`
# wrapper is required until dialout group membership is active in the
# interactive shell's supplementary groups.

# Flash the latest release build. Rebuilds first.
flash: build-firmware
    sg dialout -c "espflash flash --port {{PORT}} {{firmware_elf}}"

# Monitor defmt logs from a running device (no reflash). Exits on Ctrl+C.
monitor:
    sg dialout -c "espflash monitor --port {{PORT}} --log-format defmt --elf {{firmware_elf}}"

# Flash + monitor in one recipe. `fmr` = flash-monitor-reload, the
# default inner-loop verb. Build first, then flash, then stream logs.
fmr: build-firmware
    sg dialout -c "espflash flash --monitor --log-format defmt --port {{PORT}} {{firmware_elf}}"
