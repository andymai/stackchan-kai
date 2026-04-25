{
  description = "stackchan-kai — host-side dev shell (Rust toolchain, just, espflash, tmux)";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    # `rust-overlay` provides up-to-date rust toolchains pinned via
    # `rust-toolchain.toml` or per-build selectors. Avoids drift between
    # the host shell and CI.
    rust-overlay.url = "github:oxalica/rust-overlay";
    rust-overlay.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs = { self, nixpkgs, flake-utils, rust-overlay }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ rust-overlay.overlays.default ];
        };

        # Stable host toolchain — matches what CI's `dtolnay/rust-toolchain@stable`
        # resolves to. Includes rustfmt, clippy, and llvm-tools-preview so
        # `just check` and the coverage CI job both work locally.
        hostRust = pkgs.rust-bin.stable.latest.default.override {
          extensions = [ "rust-src" "rustfmt" "clippy" "llvm-tools-preview" ];
        };
      in {
        devShells.default = pkgs.mkShell {
          name = "stackchan-kai";

          # Host-side tooling. The xtensa-esp32s3 toolchain is NOT
          # provided here — it ships out of band via espup and lives at
          # `~/export-esp.sh`. See "Limitations" below.
          packages = with pkgs; [
            hostRust
            just
            tmux
            espflash
            cargo-deny
            cargo-machete
            cargo-audit
            cargo-llvm-cov
            git
            gh
          ];

          shellHook = ''
            # Auto-source the esp toolchain when present. The flake
            # doesn't provide it (see "Limitations" comment below) but
            # if the user has run espup, we surface it for free so
            # `just fmr` works without a manual source step.
            if [ -f "$HOME/export-esp.sh" ]; then
              # shellcheck disable=SC1091
              source "$HOME/export-esp.sh"
              esp_status="auto-sourced from \$HOME/export-esp.sh"
            else
              esp_status="not installed (run espup)"
            fi

            echo "stackchan-kai dev shell"
            echo "  rust:        $(rustc --version)"
            echo "  just:        $(just --version)"
            echo "  espflash:    $(espflash --version 2>/dev/null | head -1)"
            echo "  esp xtensa:  $esp_status"
            echo
            echo "Host crates: 'just check' to run host gates."
            echo "Firmware:    'just fmr' (esp toolchain pre-sourced if installed)."
            echo
          '';
        };
      });
}

# Limitations
# -----------
# The xtensa-esp32s3 Rust toolchain is the ESP-IDF fork (`esp-rs`)
# pre-built by espup. Packaging it as a Nix overlay is feasible but
# non-trivial:
#   - The toolchain is a custom rustc + custom LLVM build (the upstream
#     rust LLVM doesn't include the xtensa target).
#   - espup distributes pre-compiled binaries; building from source
#     requires a 30+ GB LLVM checkout and several hours of compilation.
#   - Attempts at `nixpkgs-esp-dev` and similar third-party flakes exist
#     but lag behind upstream esp-hal version bumps.
#
# The pragmatic shape: provide a host-only flake that handles
# everything except firmware cross-compilation, and leave the xtensa
# toolchain to espup + `~/export-esp.sh` as documented in CLAUDE.md.
# A future PR can add an esp toolchain overlay once the upstream Nix
# story stabilises.
