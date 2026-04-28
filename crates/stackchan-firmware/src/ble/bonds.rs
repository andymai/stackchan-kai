//! Persistent BLE bond store, backed by `/sd/BONDS.BIN`.
//!
//! `BondInformation` survives reboots so a paired phone reconnects
//! straight into an encrypted link without re-running the passkey
//! dance. Each successful pairing appends to the in-memory bond
//! list trouble-host maintains; we mirror the list onto SD so the
//! next boot can re-register them via [`Stack::add_bond_information`].
//!
//! ## Wire format
//!
//! ```text
//! header (8 bytes)         : magic "BND1" + version 1 + count + 2 reserved
//! record (44 bytes each)   : ltk(16) | bdaddr(6) | irk_present(1) | irk(16) | sec_level(1) | is_bonded(1) | reserved(3)
//! ```
//!
//! All multi-byte integers are little-endian. A missing or empty
//! file is treated as "no bonds" rather than an error — that's the
//! first-boot state.

use alloc::vec::Vec;
use heapless::Vec as HVec;
use trouble_host::prelude::{BdAddr, Identity, SecurityLevel};
use trouble_host::{BondInformation, IdentityResolvingKey, LongTermKey};

use crate::storage::with_storage;

/// Maximum bonds we'll persist. trouble-host's `BI_COUNT` is 10; we
/// match that cap so the load path can register every bond without
/// truncation.
pub const MAX_BONDS: usize = 10;

/// File header magic + version. Bumping the version invalidates
/// every existing bond file (load returns empty) so a layout
/// change can't silently mis-decode old records.
const MAGIC: [u8; 4] = *b"BND1";
/// On-disk format version. Bumping forces a forget-and-rebuild on
/// the next boot — old records can't be decoded with a new layout.
const VERSION: u8 = 1;

/// Header size in bytes.
const HEADER_LEN: usize = 8;
/// Per-bond record size. See module docs for the byte layout.
const RECORD_LEN: usize = 44;

/// Load all persisted bonds. Returns an empty list on first boot,
/// missing file, or any decode failure — the firmware boots fine
/// with no bonds, just paired peers will need to re-pair.
pub async fn load_all() -> HVec<BondInformation, MAX_BONDS> {
    let bytes = match with_storage(crate::storage::Storage::read_bonds).await {
        Some(Ok(b)) => b,
        Some(Err(e)) => {
            defmt::warn!("ble: bonds: read failed ({}); treating as empty", e);
            return HVec::new();
        }
        None => {
            defmt::info!("ble: bonds: no SD mounted; bonds disabled this run");
            return HVec::new();
        }
    };
    decode(&bytes).unwrap_or_else(|reason| {
        defmt::warn!(
            "ble: bonds: decode failed ({=str}); treating as empty",
            reason
        );
        HVec::new()
    })
}

/// Persist the entire bond list atomically. Called on each
/// `PairingComplete` event with the latest snapshot from
/// `Stack::get_bond_information`.
pub async fn save_all(bonds: &[BondInformation]) {
    let encoded = encode(bonds);
    match with_storage(|s| s.write_bonds(&encoded)).await {
        Some(Ok(())) => defmt::info!("ble: bonds: persisted {=usize} bond(s)", bonds.len()),
        Some(Err(e)) => defmt::warn!("ble: bonds: write failed ({})", e),
        None => defmt::warn!("ble: bonds: no SD mounted; cannot persist"),
    }
}

/// Encode `bonds` into the on-disk layout. Up to [`MAX_BONDS`]
/// records are written; any beyond that are silently dropped (this
/// can't happen via trouble-host, which itself caps at the same
/// number, but the slice signature lets callers pass anything).
fn encode(bonds: &[BondInformation]) -> Vec<u8> {
    // `bonds.len().min(MAX_BONDS)` fits in u8 because MAX_BONDS is
    // the trouble-host BI_COUNT (10), well below 256.
    let count_usize = bonds.len().min(MAX_BONDS);
    let count = u8::try_from(count_usize).unwrap_or(0);
    let total = HEADER_LEN + count_usize * RECORD_LEN;
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(&MAGIC);
    out.push(VERSION);
    out.push(count);
    out.push(0); // reserved
    out.push(0); // reserved

    for (i, bond) in bonds.iter().take(MAX_BONDS).enumerate() {
        let ltk_bytes = bond.ltk.0.to_le_bytes();
        out.extend_from_slice(&ltk_bytes);

        let addr = bond.identity.bd_addr.into_inner();
        out.extend_from_slice(&addr);

        if let Some(irk) = bond.identity.irk {
            out.push(1);
            out.extend_from_slice(&irk.0.to_le_bytes());
        } else {
            out.push(0);
            out.extend_from_slice(&[0u8; 16]);
        }

        out.push(security_level_to_byte(bond.security_level));
        out.push(u8::from(bond.is_bonded));
        out.extend_from_slice(&[0u8; 3]); // reserved

        // Tighter per-iteration check: each loop body must emit
        // exactly RECORD_LEN bytes. The earlier modulo form was
        // always true by algebra and couldn't catch a missing
        // field.
        debug_assert_eq!(out.len(), HEADER_LEN + (i + 1) * RECORD_LEN);
    }

    debug_assert_eq!(out.len(), total);
    out
}

/// Decode the on-disk layout into a `heapless::Vec`. Returns a
/// human-readable reason string on the failure paths so the caller
/// can log it; bonds never panic-out of decode — we'd rather lose a
/// bond than refuse to boot.
fn decode(bytes: &[u8]) -> Result<HVec<BondInformation, MAX_BONDS>, &'static str> {
    if bytes.is_empty() {
        return Ok(HVec::new());
    }
    if bytes.len() < HEADER_LEN {
        return Err("file truncated below header");
    }
    if bytes[..4] != MAGIC {
        return Err("magic mismatch");
    }
    if bytes[4] != VERSION {
        return Err("unsupported version");
    }
    let count = bytes[5] as usize;
    if count > MAX_BONDS {
        return Err("count exceeds MAX_BONDS");
    }
    let expected = HEADER_LEN + count * RECORD_LEN;
    if bytes.len() < expected {
        return Err("file truncated below records");
    }

    let mut out: HVec<BondInformation, MAX_BONDS> = HVec::new();
    let mut cursor = HEADER_LEN;
    for _ in 0..count {
        let ltk_bytes: [u8; 16] = bytes[cursor..cursor + 16]
            .try_into()
            .map_err(|_| "record ltk")?;
        let ltk = LongTermKey(u128::from_le_bytes(ltk_bytes));
        cursor += 16;

        let addr_bytes: [u8; 6] = bytes[cursor..cursor + 6]
            .try_into()
            .map_err(|_| "record addr")?;
        cursor += 6;

        let irk_present = bytes[cursor];
        cursor += 1;
        let irk_raw: [u8; 16] = bytes[cursor..cursor + 16]
            .try_into()
            .map_err(|_| "record irk")?;
        cursor += 16;
        let irk = if irk_present == 0 {
            None
        } else {
            Some(IdentityResolvingKey(u128::from_le_bytes(irk_raw)))
        };

        let sec_level = security_level_from_byte(bytes[cursor]);
        cursor += 1;
        let is_bonded = bytes[cursor] != 0;
        cursor += 1;
        cursor += 3; // reserved

        let identity = Identity {
            bd_addr: BdAddr::new(addr_bytes),
            irk,
        };

        // `push` only fails when the heapless cap is hit, which we
        // already bounded via the `count > MAX_BONDS` check above.
        let _ = out.push(BondInformation {
            ltk,
            identity,
            is_bonded,
            security_level: sec_level,
        });
    }

    Ok(out)
}

/// Map a runtime [`SecurityLevel`] to the on-disk byte. Inverse of
/// [`security_level_from_byte`].
const fn security_level_to_byte(level: SecurityLevel) -> u8 {
    match level {
        SecurityLevel::NoEncryption => 0,
        SecurityLevel::Encrypted => 1,
        SecurityLevel::EncryptedAuthenticated => 2,
    }
}

/// Inverse of [`security_level_to_byte`]. Unknown values fall back
/// to [`SecurityLevel::NoEncryption`] — a corrupt bond should not
/// silently grant write privileges.
const fn security_level_from_byte(b: u8) -> SecurityLevel {
    match b {
        2 => SecurityLevel::EncryptedAuthenticated,
        1 => SecurityLevel::Encrypted,
        _ => SecurityLevel::NoEncryption,
    }
}
