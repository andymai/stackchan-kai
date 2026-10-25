//! Host-side signer for `POST /firmware/update` images.
//!
//! Wraps an ESP app image (the `espflash save-image` output for the
//! firmware ELF) in the SCFW envelope that [`stackchan_net::ota`]
//! parses, and signs it with Ed25519. A firmware built with the
//! matching `STACKCHAN_OTA_PUBLIC_KEY` accepts the result.
//!
//! Run with:
//!
//! ```bash
//! cargo run -p stackchan-sim --bin sc-ota --features ota-cli -- <subcommand>
//! ```
//!
//! Subcommands:
//! - `keygen <key-path>` — write a fresh signing seed (64 hex chars)
//!   to `<key-path>` and print the public key on stdout. Refuses to
//!   overwrite an existing file.
//! - `pubkey <key-path>` — print the public key for an existing seed,
//!   the value to export as `STACKCHAN_OTA_PUBLIC_KEY` before building.
//! - `sign <key-path> <app.bin> <out.scfw>` — seal and sign an app
//!   image, then run it through the firmware's own parse + verify
//!   path before writing it.

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

use std::fmt::Write as _;
use std::io::Write as _;
use std::process::ExitCode;

use ed25519_compact::{KeyPair, Seed};
use stackchan_net::ota::{
    MAX_OTA_PAYLOAD_BYTES, OTA_HEADER_LEN, OTA_SIGNATURE_LEN, build_header, parse_and_verify,
};

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
    let arg = |i: usize, what: &str| {
        args.get(i)
            .map(String::as_str)
            .ok_or_else(|| format!("missing {what}\n\n{USAGE}"))
    };
    match args.first().map(String::as_str) {
        Some("keygen") => keygen(arg(1, "key path")?),
        Some("pubkey") => {
            let keypair = read_keypair(arg(1, "key path")?)?;
            println!("{}", encode_hex(&*keypair.pk));
            Ok(())
        }
        Some("sign") => sign(
            arg(1, "key path")?,
            arg(2, "app image path")?,
            arg(3, "output path")?,
        ),
        Some(other) => Err(format!("unknown subcommand {other:?}\n\n{USAGE}")),
        None => Err(format!("missing subcommand\n\n{USAGE}")),
    }
}

const USAGE: &str = "usage:
  sc-ota keygen <key-path>
  sc-ota pubkey <key-path>
  sc-ota sign <key-path> <app.bin> <out.scfw>";

/// First byte of every ESP app image header (`ESP_IMAGE_HEADER_MAGIC`).
/// Checked so a stray ELF or dashboard bundle fails here instead of
/// being flashed into the inactive slot.
const ESP_IMAGE_MAGIC: u8 = 0xE9;

fn keygen(path: &str) -> Result<(), String> {
    let mut bytes = [0u8; Seed::BYTES];
    getrandom::fill(&mut bytes).map_err(|e| format!("reading the OS RNG: {e}"))?;
    let seed = Seed::new(bytes);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options
        .open(path)
        .map_err(|e| format!("creating {path}: {e}"))?;
    writeln!(file, "{}", encode_hex(&*seed)).map_err(|e| format!("writing {path}: {e}"))?;
    let keypair = KeyPair::from_seed(seed);
    eprintln!(
        "wrote signing key to {path}; anyone holding it can sign firmware this device accepts"
    );
    println!("{}", encode_hex(&*keypair.pk));
    Ok(())
}

fn sign(key_path: &str, app_path: &str, out_path: &str) -> Result<(), String> {
    if same_file(key_path, out_path) {
        return Err(format!(
            "output {out_path} is the signing key; refusing to overwrite it"
        ));
    }
    let keypair = read_keypair(key_path)?;
    let payload = std::fs::read(app_path).map_err(|e| format!("reading {app_path}: {e}"))?;
    let image = seal(&payload, &keypair).map_err(|e| format!("{app_path}: {e}"))?;
    std::fs::write(out_path, &image).map_err(|e| format!("writing {out_path}: {e}"))?;
    eprintln!(
        "signed {} -> {out_path} ({} byte payload, {} byte image)",
        app_path,
        payload.len(),
        image.len()
    );
    Ok(())
}

/// True when both paths exist and name the same file, through
/// symlinks or hard links.
fn same_file(a: &str, b: &str) -> bool {
    let (Ok(meta_a), Ok(meta_b)) = (std::fs::metadata(a), std::fs::metadata(b)) else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        meta_a.dev() == meta_b.dev() && meta_a.ino() == meta_b.ino()
    }
    #[cfg(not(unix))]
    {
        let _ = (meta_a, meta_b);
        std::fs::canonicalize(a).ok() == std::fs::canonicalize(b).ok()
    }
}

fn read_keypair(path: &str) -> Result<KeyPair, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("reading {path}: {e}"))?;
    let seed = decode_hex32(text.trim()).map_err(|e| format!("{path}: {e}"))?;
    Ok(KeyPair::from_seed(Seed::new(seed)))
}

/// Build `header ‖ payload ‖ signature` and prove the firmware's
/// verifier accepts it before returning.
fn seal(payload: &[u8], keypair: &KeyPair) -> Result<Vec<u8>, String> {
    if payload.first() != Some(&ESP_IMAGE_MAGIC) {
        return Err(
            "not an ESP app image (first byte isn't 0xE9); sign the `espflash save-image` output, not the ELF"
                .to_string(),
        );
    }
    let len = u32::try_from(payload.len())
        .ok()
        .filter(|&n| n <= MAX_OTA_PAYLOAD_BYTES)
        .ok_or_else(|| {
            format!(
                "{} bytes exceeds the {MAX_OTA_PAYLOAD_BYTES}-byte OTA payload cap",
                payload.len()
            )
        })?;
    let signature = keypair.sk.sign(payload, None);
    let mut image = Vec::with_capacity(OTA_HEADER_LEN + payload.len() + OTA_SIGNATURE_LEN);
    image.extend_from_slice(&build_header(len));
    image.extend_from_slice(payload);
    image.extend_from_slice(&*signature);
    parse_and_verify(&image, &keypair.pk)
        .map_err(|e| format!("internal: sealed image fails verification: {e:?}"))?;
    Ok(image)
}

fn decode_hex32(text: &str) -> Result<[u8; 32], String> {
    if text.len() != 64 {
        return Err(format!("expected 64 hex characters, found {}", text.len()));
    }
    let mut out = [0u8; 32];
    let (pairs, _) = text.as_bytes().as_chunks::<2>();
    for (byte, pair) in out.iter_mut().zip(pairs) {
        let pair = std::str::from_utf8(pair).map_err(|_| "key isn't ASCII hex".to_string())?;
        *byte = u8::from_str_radix(pair, 16).map_err(|_| format!("{pair:?} isn't hex"))?;
    }
    Ok(out)
}

fn encode_hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut out, b| {
            let _ = write!(out, "{b:02x}");
            out
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_keypair() -> KeyPair {
        KeyPair::from_seed(Seed::new([7; 32]))
    }

    fn app_image(len: usize) -> Vec<u8> {
        let mut payload = vec![0xA5; len];
        payload[0] = ESP_IMAGE_MAGIC;
        payload
    }

    #[test]
    fn sealed_image_passes_firmware_verifier() {
        let keypair = test_keypair();
        let payload = app_image(1024);
        let image = seal(&payload, &keypair).unwrap();
        assert_eq!(parse_and_verify(&image, &keypair.pk).unwrap(), &payload[..]);
    }

    #[test]
    fn sealed_image_fails_under_another_key() {
        let image = seal(&app_image(64), &test_keypair()).unwrap();
        let other = KeyPair::from_seed(Seed::new([8; 32]));
        assert!(parse_and_verify(&image, &other.pk).is_err());
    }

    #[test]
    fn tampered_payload_fails_verification() {
        let keypair = test_keypair();
        let mut image = seal(&app_image(64), &keypair).unwrap();
        image[OTA_HEADER_LEN + 10] ^= 0x01;
        assert!(parse_and_verify(&image, &keypair.pk).is_err());
    }

    #[test]
    fn rejects_elf_input() {
        let elf = b"\x7fELF\x01\x01\x01".to_vec();
        assert!(seal(&elf, &test_keypair()).is_err());
    }

    #[test]
    fn rejects_payload_over_cap() {
        let oversized = app_image(MAX_OTA_PAYLOAD_BYTES as usize + 1);
        assert!(seal(&oversized, &test_keypair()).is_err());
    }

    #[test]
    fn hex_round_trips_and_rejects_malformed_keys() {
        let bytes: [u8; 32] = core::array::from_fn(|i| u8::try_from(i * 7).unwrap());
        assert_eq!(decode_hex32(&encode_hex(&bytes)).unwrap(), bytes);
        assert!(decode_hex32("abcd").is_err());
        assert!(decode_hex32(&"zz".repeat(32)).is_err());
    }

    #[test]
    fn sign_refuses_to_write_over_the_key() {
        let dir = std::env::temp_dir().join(format!("sc-ota-sign-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let key = dir.join("ota.key");
        let link = dir.join("link.key");
        let app = dir.join("app.bin");
        let key_str = key.to_str().unwrap();
        keygen(key_str).unwrap();
        std::fs::hard_link(&key, &link).unwrap();
        std::fs::write(&app, app_image(64)).unwrap();
        let before = std::fs::read_to_string(&key).unwrap();
        for out in [&key, &link] {
            assert!(sign(key_str, app.to_str().unwrap(), out.to_str().unwrap()).is_err());
        }
        assert_eq!(std::fs::read_to_string(&key).unwrap(), before);
        let out = dir.join("fw.scfw");
        sign(key_str, app.to_str().unwrap(), out.to_str().unwrap()).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn keygen_refuses_to_overwrite() {
        let path = std::env::temp_dir().join(format!("sc-ota-keygen-{}", std::process::id()));
        let path_str = path.to_str().unwrap();
        let _ = std::fs::remove_file(&path);
        keygen(path_str).unwrap();
        let first = std::fs::read_to_string(&path).unwrap();
        assert!(keygen(path_str).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), first);
        assert!(read_keypair(path_str).is_ok());
        let _ = std::fs::remove_file(&path);
    }
}
