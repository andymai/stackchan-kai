//! Encoder / decoder for the firmware's persistent crash latch.
//!
//! The firmware captures panic info into a fixed-size byte buffer
//! placed in RTC fast RAM (preserved across `software_reset`,
//! watchdog timeouts, and the bootloader handoff via
//! `#[esp_hal::ram(unstable(rtc_fast, persistent))]`). On the next
//! boot, the same buffer is decoded back into a [`CrashSnapshot`]
//! and saved to `/sd/CRASH.LOG`.
//!
//! This module owns the byte-level layout and the magic / checksum
//! validation. The firmware crate is `xtensa-esp32s3-none-elf`-only,
//! so the actual `static mut [u8; LATCH_SIZE]` and the
//! `#[panic_handler]` glue live there. Putting the codec here keeps
//! it host-testable — the panic / boot pair is non-trivial enough
//! that a layout typo would silently lose every crash report.
//!
//! ## Layout
//!
//! ```text
//! offset  size  field
//!  0       4    magic       `u32` little-endian, [`MAGIC`]
//!  4       4    line        `u32` panic location line number
//!  8       2    msg_len     `u16` little-endian, bytes of `message` to read
//! 10       2    file_len    `u16` little-endian, bytes of `file` to read
//! 12     256    message     `[u8; 256]` truncated panic message
//! 268    112    file        `[u8; 112]` truncated source file path
//! 380      4    checksum    `u32` sum-of-bytes XOR over [0, 380)
//! ```

use core::fmt::Write as _;

/// Magic header — the bit pattern that distinguishes a real crash
/// snapshot from random RTC RAM contents on first boot. Spelled
/// `0xC1A5_DEAD` (`CrAS Dead`) so a hex dump of the latch is
/// self-describing.
pub const MAGIC: u32 = 0xC1A5_DEAD;

/// Maximum panic-message bytes captured. Longer messages are
/// silently truncated.
pub const MSG_CAP: usize = 256;

/// Maximum source-file-path bytes captured. Same truncation policy
/// as [`MSG_CAP`].
pub const FILE_CAP: usize = 112;

/// Total latch size in bytes. Layout described in the module-level
/// docs.
pub const LATCH_SIZE: usize = 384;

/// Byte offset of the magic header within the latch buffer.
const MAGIC_OFFSET: usize = 0;
/// Byte offset of the panic-location line number.
const LINE_OFFSET: usize = 4;
/// Byte offset of the message-length word (little-endian `u16`).
const MSG_LEN_OFFSET: usize = 8;
/// Byte offset of the file-length word (little-endian `u16`).
const FILE_LEN_OFFSET: usize = 10;
/// Byte offset where the message content begins.
const MSG_OFFSET: usize = 12;
/// Byte offset where the source-file-path content begins.
const FILE_OFFSET: usize = MSG_OFFSET + MSG_CAP;
/// Byte offset of the trailing checksum word.
const CHECKSUM_OFFSET: usize = FILE_OFFSET + FILE_CAP;

const _: () = {
    assert!(CHECKSUM_OFFSET + 4 == LATCH_SIZE);
};

/// Decoded crash snapshot returned from [`decode`]. Owns its strings
/// via fixed-size buffers so the consumer doesn't need to allocate.
#[derive(Debug, Clone, Copy)]
pub struct CrashSnapshot {
    /// Panic message bytes (UTF-8 best-effort; truncated to fit
    /// [`MSG_CAP`]).
    pub message: [u8; MSG_CAP],
    /// Number of valid bytes in [`Self::message`].
    pub message_len: usize,
    /// Source file path bytes (UTF-8 best-effort; truncated to fit
    /// [`FILE_CAP`]).
    pub file: [u8; FILE_CAP],
    /// Number of valid bytes in [`Self::file`].
    pub file_len: usize,
    /// Source line number, or `0` if `PanicInfo::location()` was
    /// `None` at the time of the panic.
    pub line: u32,
}

impl CrashSnapshot {
    /// Borrow the panic message as a string slice. Returns the
    /// canonical placeholder when the bytes don't decode as UTF-8 —
    /// callers can still surface the bytes via the raw `message`
    /// field if they need to.
    #[must_use]
    pub fn message_str(&self) -> &str {
        core::str::from_utf8(&self.message[..self.message_len]).unwrap_or("<non-utf8>")
    }

    /// Borrow the source file path as a string slice; same fallback
    /// as [`Self::message_str`].
    #[must_use]
    pub fn file_str(&self) -> &str {
        core::str::from_utf8(&self.file[..self.file_len]).unwrap_or("<non-utf8>")
    }
}

/// Write the message + location + checksum into `latch`. Truncates
/// silently if `message` or `file` exceed [`MSG_CAP`] / [`FILE_CAP`].
///
/// Used by the firmware's `#[panic_handler]`. Splitting the
/// formatter from the byte writer is intentional — the panic
/// handler renders `core::fmt::Arguments` into a stack buffer via
/// [`format_into`], then calls this with the resulting slice.
pub fn encode(latch: &mut [u8; LATCH_SIZE], message: &[u8], file: &[u8], line: u32) {
    latch[MAGIC_OFFSET..MAGIC_OFFSET + 4].copy_from_slice(&MAGIC.to_le_bytes());
    latch[LINE_OFFSET..LINE_OFFSET + 4].copy_from_slice(&line.to_le_bytes());

    let msg_n = message.len().min(MSG_CAP);
    let file_n = file.len().min(FILE_CAP);
    #[allow(
        clippy::cast_possible_truncation,
        reason = "msg_n / file_n are clamped to MSG_CAP / FILE_CAP, both <= u16::MAX"
    )]
    {
        latch[MSG_LEN_OFFSET..MSG_LEN_OFFSET + 2].copy_from_slice(&(msg_n as u16).to_le_bytes());
        latch[FILE_LEN_OFFSET..FILE_LEN_OFFSET + 2].copy_from_slice(&(file_n as u16).to_le_bytes());
    }

    latch[MSG_OFFSET..MSG_OFFSET + MSG_CAP].fill(0);
    latch[MSG_OFFSET..MSG_OFFSET + msg_n].copy_from_slice(&message[..msg_n]);

    latch[FILE_OFFSET..FILE_OFFSET + FILE_CAP].fill(0);
    latch[FILE_OFFSET..FILE_OFFSET + file_n].copy_from_slice(&file[..file_n]);

    let cs = compute_checksum(&latch[..CHECKSUM_OFFSET]);
    latch[CHECKSUM_OFFSET..CHECKSUM_OFFSET + 4].copy_from_slice(&cs.to_le_bytes());
}

/// Validate magic + checksum and decode `latch` into a snapshot.
///
/// Returns `None` if the magic doesn't match (cold boot, RTC RAM
/// uninitialised) or if the checksum doesn't validate (partial
/// corruption between panic and read-back). The caller is
/// responsible for clearing the magic after a successful read so
/// the same crash isn't replayed on the next boot.
#[must_use]
pub fn decode(latch: &[u8; LATCH_SIZE]) -> Option<CrashSnapshot> {
    let mut magic_bytes = [0u8; 4];
    magic_bytes.copy_from_slice(&latch[MAGIC_OFFSET..MAGIC_OFFSET + 4]);
    if u32::from_le_bytes(magic_bytes) != MAGIC {
        return None;
    }

    let mut cs_bytes = [0u8; 4];
    cs_bytes.copy_from_slice(&latch[CHECKSUM_OFFSET..CHECKSUM_OFFSET + 4]);
    if u32::from_le_bytes(cs_bytes) != compute_checksum(&latch[..CHECKSUM_OFFSET]) {
        return None;
    }

    let mut msg_len_bytes = [0u8; 2];
    msg_len_bytes.copy_from_slice(&latch[MSG_LEN_OFFSET..MSG_LEN_OFFSET + 2]);
    let msg_len = (u16::from_le_bytes(msg_len_bytes) as usize).min(MSG_CAP);
    let mut file_len_bytes = [0u8; 2];
    file_len_bytes.copy_from_slice(&latch[FILE_LEN_OFFSET..FILE_LEN_OFFSET + 2]);
    let file_len = (u16::from_le_bytes(file_len_bytes) as usize).min(FILE_CAP);
    let mut line_bytes = [0u8; 4];
    line_bytes.copy_from_slice(&latch[LINE_OFFSET..LINE_OFFSET + 4]);

    let mut message = [0u8; MSG_CAP];
    message.copy_from_slice(&latch[MSG_OFFSET..MSG_OFFSET + MSG_CAP]);
    let mut file = [0u8; FILE_CAP];
    file.copy_from_slice(&latch[FILE_OFFSET..FILE_OFFSET + FILE_CAP]);

    Some(CrashSnapshot {
        message,
        message_len: msg_len,
        file,
        file_len,
        line: u32::from_le_bytes(line_bytes),
    })
}

/// Zero out the magic so the next [`decode`] call returns `None`.
/// Used after a successful read to avoid replaying the same crash
/// on every subsequent boot.
pub fn clear_magic(latch: &mut [u8; LATCH_SIZE]) {
    latch[MAGIC_OFFSET..MAGIC_OFFSET + 4].copy_from_slice(&[0u8; 4]);
}

/// Sum-of-bytes XOR'd into a `u32` with byte-position rotation.
///
/// Strong enough to flag single-byte corruption — any flipped bit
/// shifts the rotated XOR away from the stored value. Not a
/// cryptographic hash; the expected failure mode is "RTC RAM kept
/// the magic across reset but the body got partially overwritten by
/// the bootloader scratchpad."
#[must_use]
pub fn compute_checksum(bytes: &[u8]) -> u32 {
    let mut acc: u32 = 0;
    for (i, &b) in bytes.iter().enumerate() {
        #[allow(
            clippy::cast_possible_truncation,
            reason = "the rotation amount is `i % 32`; the cast to u32 \
                      cannot lose information"
        )]
        let shifted = u32::from(b).rotate_left((i % 32) as u32);
        acc ^= shifted;
    }
    acc
}

/// Render `args` into `buf`, silently truncating past the buffer's
/// capacity. Returns the number of bytes actually written.
///
/// The firmware's panic handler uses this to flatten
/// `PanicInfo::message()` into a stack buffer suitable for
/// [`encode`] without allocating — `format!` requires a working
/// allocator, which is exactly what may have just panicked.
pub fn format_into(buf: &mut [u8], args: core::fmt::Arguments<'_>) -> usize {
    let mut sink = Truncate { buf, len: 0 };
    let _ = sink.write_fmt(args);
    sink.len
}

/// `core::fmt::Write` sink that captures bytes into a fixed
/// caller-owned slice, silently dropping anything past the slice's
/// capacity. Backs [`format_into`].
struct Truncate<'a> {
    /// Output slice; bytes are written from the start.
    buf: &'a mut [u8],
    /// Number of bytes written so far. Read by [`format_into`] to
    /// return the captured length.
    len: usize,
}

impl core::fmt::Write for Truncate<'_> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let cap = self.buf.len();
        for &b in s.as_bytes() {
            if self.len < cap {
                self.buf[self.len] = b;
                self.len += 1;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::missing_docs_in_private_items,
    reason = "test-only: fixtures are constructed in-line and assertions \
              guarantee the unwrap branch is unreachable for valid inputs"
)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_short_message() {
        let mut buf = [0u8; LATCH_SIZE];
        encode(&mut buf, b"boom", b"src/main.rs", 42);
        let snap = decode(&buf).expect("valid latch decodes");
        assert_eq!(snap.message_str(), "boom");
        assert_eq!(snap.file_str(), "src/main.rs");
        assert_eq!(snap.line, 42);
    }

    #[test]
    fn round_trip_no_location() {
        // A panic with `info.location() == None` writes line=0 and
        // file_len=0; decode returns those values verbatim so the
        // SD-side logger can render `<unknown>:0` deliberately.
        let mut buf = [0u8; LATCH_SIZE];
        encode(&mut buf, b"oops", b"", 0);
        let snap = decode(&buf).expect("valid latch decodes");
        assert_eq!(snap.message_str(), "oops");
        assert_eq!(snap.file_str(), "");
        assert_eq!(snap.line, 0);
    }

    #[test]
    fn empty_buffer_is_rejected() {
        let buf = [0u8; LATCH_SIZE];
        assert!(decode(&buf).is_none());
    }

    #[test]
    fn random_bytes_with_wrong_magic_rejected() {
        let mut buf = [0u8; LATCH_SIZE];
        for (i, slot) in buf.iter_mut().enumerate() {
            #[allow(clippy::cast_possible_truncation)]
            {
                *slot = (i & 0xFF) as u8;
            }
        }
        // Even though the body has data, the magic header almost
        // certainly doesn't match — guards against treating
        // pre-init RAM as a valid crash snapshot.
        assert!(decode(&buf).is_none());
    }

    #[test]
    fn corrupt_body_after_valid_magic_rejected() {
        let mut buf = [0u8; LATCH_SIZE];
        encode(&mut buf, b"boom", b"src/main.rs", 42);
        buf[MSG_OFFSET] ^= 0x01;
        assert!(decode(&buf).is_none());
    }

    #[test]
    fn message_longer_than_cap_is_silently_truncated() {
        let mut buf = [0u8; LATCH_SIZE];
        let long = vec![b'x'; MSG_CAP * 2];
        encode(&mut buf, &long, b"src/main.rs", 1);
        let snap = decode(&buf).expect("valid latch decodes");
        assert_eq!(snap.message_len, MSG_CAP);
        assert!(snap.message_str().bytes().all(|b| b == b'x'));
    }

    #[test]
    fn file_longer_than_cap_is_silently_truncated() {
        let mut buf = [0u8; LATCH_SIZE];
        let long = vec![b'p'; FILE_CAP * 2];
        encode(&mut buf, b"boom", &long, 1);
        let snap = decode(&buf).expect("valid latch decodes");
        assert_eq!(snap.file_len, FILE_CAP);
    }

    #[test]
    fn format_into_truncates_silently() {
        let mut buf = [0u8; 8];
        let n = format_into(&mut buf, format_args!("hello world!"));
        assert_eq!(n, 8);
        assert_eq!(&buf, b"hello wo");
    }

    #[test]
    fn format_into_handles_full_args() {
        let mut buf = [0u8; 64];
        let n = format_into(&mut buf, format_args!("panic at {}:{}", "src/main.rs", 42));
        assert_eq!(&buf[..n], b"panic at src/main.rs:42");
    }

    #[test]
    fn checksum_changes_when_any_byte_changes() {
        let mut buf = [0u8; LATCH_SIZE];
        encode(&mut buf, b"boom", b"src/main.rs", 42);
        let original_cs = compute_checksum(&buf[..CHECKSUM_OFFSET]);
        for offset in [
            MSG_OFFSET,
            MSG_OFFSET + 1,
            FILE_OFFSET,
            LINE_OFFSET,
            MAGIC_OFFSET,
        ] {
            let saved = buf[offset];
            buf[offset] ^= 0x01;
            assert_ne!(
                compute_checksum(&buf[..CHECKSUM_OFFSET]),
                original_cs,
                "checksum collision when flipping byte at offset {offset}",
            );
            buf[offset] = saved;
        }
    }

    #[test]
    fn clear_magic_makes_subsequent_decode_return_none() {
        let mut buf = [0u8; LATCH_SIZE];
        encode(&mut buf, b"boom", b"src/main.rs", 42);
        assert!(decode(&buf).is_some());
        clear_magic(&mut buf);
        assert!(decode(&buf).is_none());
    }

    #[test]
    fn re_encode_after_clear_round_trips() {
        let mut buf = [0u8; LATCH_SIZE];
        encode(&mut buf, b"first", b"a.rs", 1);
        clear_magic(&mut buf);
        encode(&mut buf, b"second", b"b.rs", 2);
        let snap = decode(&buf).expect("valid latch decodes");
        assert_eq!(snap.message_str(), "second");
        assert_eq!(snap.file_str(), "b.rs");
        assert_eq!(snap.line, 2);
    }
}
