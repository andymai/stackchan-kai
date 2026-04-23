# stackchan-rs development tasks.
#
# Install just: https://github.com/casey/just
# Install espup + esp toolchain:
#   cargo install espup
#   espup install
#   source $HOME/export-esp.sh

set shell := ["bash", "-cu"]

# Default: list available recipes.
default:
    @just --list

# Host-side checks (core + sim + axp2101). Fast.
check:
    cargo fmt --check
    cargo clippy --workspace --exclude stackchan-firmware --all-features -- -D warnings
    cargo test --workspace --exclude stackchan-firmware --all-features

# Firmware-side compile check (requires esp toolchain sourced).
check-firmware:
    cargo check -p stackchan-firmware --target xtensa-esp32s3-none-elf

# Full build of the firmware binary.
build-firmware:
    cargo build -p stackchan-firmware --release --target xtensa-esp32s3-none-elf

# Flash the firmware to a connected CoreS3 at $PORT (defaults to /dev/ttyACM1).
flash PORT='/dev/ttyACM1':
    cargo run -p stackchan-firmware --release --target xtensa-esp32s3-none-elf -- --port {{PORT}}

# Monitor defmt-rtt logs from a connected CoreS3.
monitor PORT='/dev/ttyACM1':
    espflash monitor --port {{PORT}}

# Run everything CI runs (host side).
ci: check
    cargo deny check
    cargo doc --no-deps --workspace --exclude stackchan-firmware --all-features
