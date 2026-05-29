//! Host-side `STACKCHAN.RON` craft + validate tool.
//!
//! A thin shell over the public `stackchan_net::config` API so an
//! operator can build and sanity-check a boot config offline before
//! copying it to the SD card — the firmware runs the same
//! `validate_for_disk` gate at boot, so a config that passes `validate`
//! here will load on device.
//!
//! Run with:
//!
//! ```bash
//! cargo run -p stackchan-sim --bin sc-config --features config-cli -- <subcommand>
//! ```
//!
//! Subcommands:
//! - `validate <path>` — parse + strict-validate an existing file.
//!   Exits non-zero with the typed error on failure.
//! - `list` — print the accepted palette + face-geometry wire names,
//!   sourced from `stackchan-core` so they can never drift from the
//!   validator.
//! - `template [--palette <name>] [--face-geometry <name>] [--ssid <ssid>]`
//!   — emit a default config with the appearance block populated, to
//!   stdout. A placeholder SSID is written unless `--ssid` overrides it,
//!   since the disk validator rejects an empty SSID.

// Dev-tool binary: relax the workspace's library-grade lints so the
// arg-dispatch + stdout-printing one-offs don't drown the file in
// noise. Library code (everything outside `src/bin/`) stays strict.
#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::expect_used,
    clippy::unwrap_used,
    missing_docs,
    clippy::missing_docs_in_private_items
)]

use std::process::ExitCode;

use stackchan_core::{FaceGeometry, Palette};
use stackchan_net::config::{AppearanceConfig, Config, WifiConfig, parse_ron, render_ron};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("error: {msg}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<(), String> {
    match args.first().map(String::as_str) {
        Some("validate") => validate(args.get(1).map(String::as_str)),
        Some("list") => {
            list();
            Ok(())
        }
        Some("template") => template(&args[1..]),
        Some(other) => Err(format!("unknown subcommand {other:?}\n\n{USAGE}")),
        None => Err(format!("missing subcommand\n\n{USAGE}")),
    }
}

const USAGE: &str = "usage:
  sc-config validate <path>
  sc-config list
  sc-config template [--palette <name>] [--face-geometry <name>]";

fn validate(path: Option<&str>) -> Result<(), String> {
    let path = path.ok_or_else(|| format!("validate needs a file path\n\n{USAGE}"))?;
    let text = std::fs::read_to_string(path).map_err(|e| format!("reading {path}: {e}"))?;
    let config = parse_ron(&text).map_err(|e| e.to_string())?;
    println!("{path}: valid");
    println!("  hostname:      {}", config.mdns.hostname);
    println!(
        "  wifi.ssid:     {}",
        if config.wifi.ssid.is_empty() {
            "(offline)"
        } else {
            &config.wifi.ssid
        }
    );
    println!(
        "  palette:       {}",
        appearance_or_default(&config.appearance.palette)
    );
    println!(
        "  face_geometry: {}",
        appearance_or_default(&config.appearance.face_geometry)
    );
    Ok(())
}

const fn appearance_or_default(wire: &str) -> &str {
    if wire.is_empty() { "(unpinned)" } else { wire }
}

fn list() {
    println!("palettes:");
    for p in Palette::ALL {
        println!("  {}", p.wire_str());
    }
    println!("face geometries:");
    for g in FaceGeometry::ALL {
        println!("  {}", g.wire_str());
    }
}

/// Placeholder SSID written into a fresh template.
///
/// `validate_for_disk` — the gate both this tool and the firmware run
/// before accepting a file — rejects an empty SSID, so a template can't
/// emit `Config::default()` verbatim (its empty SSID is the in-memory
/// "no Wi-Fi" sentinel, never a disk value). A placeholder keeps the
/// emitted file loadable; the operator overwrites it with `--ssid` or a
/// hand-edit.
const PLACEHOLDER_SSID: &str = "my-wifi";

fn template(args: &[String]) -> Result<(), String> {
    let mut palette = String::new();
    let mut face_geometry = String::new();
    let mut ssid = PLACEHOLDER_SSID.to_string();
    let mut iter = args.iter();
    while let Some(flag) = iter.next() {
        match flag.as_str() {
            "--palette" => {
                let name = iter
                    .next()
                    .ok_or_else(|| "--palette needs a value".to_string())?;
                if Palette::from_wire_str(name).is_none() {
                    return Err(format!("unknown palette {name:?}; {}", palette_names()));
                }
                palette.clone_from(name);
            }
            "--face-geometry" => {
                let name = iter
                    .next()
                    .ok_or_else(|| "--face-geometry needs a value".to_string())?;
                if FaceGeometry::from_wire_str(name).is_none() {
                    return Err(format!(
                        "unknown face geometry {name:?}; {}",
                        geometry_names()
                    ));
                }
                face_geometry.clone_from(name);
            }
            "--ssid" => {
                let value = iter
                    .next()
                    .ok_or_else(|| "--ssid needs a value".to_string())?;
                ssid.clone_from(value);
            }
            other => return Err(format!("unknown flag {other:?}\n\n{USAGE}")),
        }
    }

    let config = Config {
        wifi: WifiConfig {
            ssid,
            ..Default::default()
        },
        appearance: AppearanceConfig {
            palette,
            face_geometry,
        },
        ..Config::default()
    };
    let rendered = render_ron(&config).map_err(|e| e.to_string())?;
    // Re-validate our own output through the disk gate so the tool can
    // never emit a file the firmware would reject at boot.
    parse_ron(&rendered).map_err(|e| format!("internal: emitted invalid template: {e}"))?;
    print!("{rendered}");
    Ok(())
}

fn palette_names() -> String {
    let names: Vec<&str> = Palette::ALL.iter().map(|p| p.wire_str()).collect();
    format!("valid: {}", names.join(", "))
}

fn geometry_names() -> String {
    let names: Vec<&str> = FaceGeometry::ALL.iter().map(|g| g.wire_str()).collect();
    format!("valid: {}", names.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_output_parses_back_through_disk_gate() {
        template(&[
            "--palette".to_string(),
            "cute".to_string(),
            "--face-geometry".to_string(),
            "chibi".to_string(),
        ])
        .expect("template with valid appearance succeeds");
    }

    #[test]
    fn template_default_ssid_passes_disk_validation() {
        template(&[]).expect("bare template emits a loadable config");
    }

    #[test]
    fn template_rejects_unknown_palette() {
        let err = template(&["--palette".to_string(), "rainbow".to_string()])
            .expect_err("unknown palette must error");
        assert!(
            err.contains("rainbow"),
            "error mentions the bad name: {err}"
        );
    }

    #[test]
    fn template_rejects_unknown_face_geometry() {
        let err = template(&["--face-geometry".to_string(), "blocky".to_string()])
            .expect_err("unknown face geometry must error");
        assert!(err.contains("blocky"), "error mentions the bad name: {err}");
    }

    #[test]
    fn validate_rejects_malformed_ron() {
        let dir = std::env::temp_dir();
        let path = dir.join("sc-config-test-malformed.ron");
        std::fs::write(&path, "this is not ron").expect("write temp file");
        let result = validate(Some(path.to_str().expect("utf-8 temp path")));
        let _ = std::fs::remove_file(&path);
        assert!(result.is_err(), "malformed RON must be rejected");
    }

    #[test]
    fn validate_rejects_unknown_palette_in_file() {
        let config = Config {
            wifi: stackchan_net::config::WifiConfig {
                ssid: "net".to_string(),
                ..Default::default()
            },
            appearance: AppearanceConfig {
                palette: "rainbow".to_string(),
                ..Default::default()
            },
            ..Config::default()
        };
        let rendered = render_ron(&config).expect("render config");
        let dir = std::env::temp_dir();
        let path = dir.join("sc-config-test-bad-palette.ron");
        std::fs::write(&path, rendered).expect("write temp file");
        let result = validate(Some(path.to_str().expect("utf-8 temp path")));
        let _ = std::fs::remove_file(&path);
        assert!(result.is_err(), "unknown palette must be rejected");
    }
}
