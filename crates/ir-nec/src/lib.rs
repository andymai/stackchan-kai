//! # ir-nec
//!
//! `no_std` NEC-protocol IR-remote decoder and encoder with no hardware
//! dependency. On the receive side, consumers pass in a slice of
//! [`Pulse`] timings (captured however they like: esp-hal's RMT
//! peripheral, a GPIO ISR, a simulator) and get back an
//! [`Option<NecCommand>`]. On the transmit side, [`NecCommand::encode`]
//! produces a fixed-size [`Pulse`] array the caller can feed straight
//! into the RMT TX buffer.
//!
//! ## NEC frame shape
//!
//! ```text
//! 9.0 ms mark  4.5 ms space  <32 data bits>  0.56 ms mark (stop)
//! ```
//!
//! Each data bit is a `560 µs` mark followed by a space: short
//! (`560 µs`) = `0`, long (`1.69 ms`) = `1`. Data bits are transmitted
//! LSB-first within each byte: `addr_low, addr_high, cmd, ~cmd`. The
//! `cmd` / `~cmd` pair is a checksum — receivers validate by `XOR`ing
//! the two bytes and requiring `0xFF`.
//!
//! Some remotes use the "extended" NEC variant where the address
//! bytes carry arbitrary data rather than the old `addr / ~addr`
//! complementary pair. This decoder does **not** validate the address
//! checksum; it treats the address as a plain `u16` so both variants
//! work.
//!
//! ## Timing tolerance
//!
//! Real-world IR receivers add ±10% jitter. [`TOLERANCE_US`] widens
//! every "is this close to X" comparison so typical decodes succeed.

#![no_std]
#![deny(unsafe_code)]

/// One pulse interval, as captured by a hardware-timer peripheral.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pulse {
    /// Logical IR-light level during this interval.
    ///
    /// Most IR receivers output an *active-low* signal (IRM56384 on
    /// CoreS3 inverts internally), so "mark" (IR on) surfaces as
    /// `low` at the GPIO. This module treats `level = true` as
    /// "IR on" (mark) — callers that see active-low signals should
    /// pre-invert.
    pub level: bool,
    /// Duration of the pulse in microseconds.
    pub duration_us: u32,
}

/// Decoded NEC frame.
///
/// The NEC protocol transmits 4 bytes over the air: `address_low`,
/// `address_high`, `command`, `~command`. We expose address as a
/// single `u16` (low byte first, LSB per-byte-first-on-wire) and
/// command as a `u8`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NecCommand {
    /// 16-bit address the remote is broadcasting at.
    ///
    /// For classic NEC remotes this is `addr_low | (~addr_low << 8)`;
    /// for extended NEC it's an arbitrary 16-bit value.
    pub address: u16,
    /// 8-bit command code.
    pub command: u8,
}

/// Absolute timing tolerance for pulse comparisons, in microseconds.
///
/// Applied as a symmetric ±window around each spec value. Wide enough
/// that cheap IR receivers with ~10% sample jitter decode reliably,
/// narrow enough that ambient noise doesn't get mistaken for a frame.
pub const TOLERANCE_US: u32 = 200;

/// Expected length of the initial mark (AGC burst).
const PREAMBLE_MARK_US: u32 = 9_000;
/// Expected length of the initial space (AGC gap).
const PREAMBLE_SPACE_US: u32 = 4_500;
/// Expected length of each data-bit mark (constant across 0 and 1 bits).
const BIT_MARK_US: u32 = 560;
/// Expected space length for a `0` bit (short space).
const BIT_ZERO_SPACE_US: u32 = 560;
/// Expected space length for a `1` bit (long space).
const BIT_ONE_SPACE_US: u32 = 1_690;

/// Number of data bits the NEC protocol transmits per frame.
const DATA_BITS: usize = 32;

/// Number of pulses a complete NEC frame produces:
/// 2 (preamble) + 2 × 32 (data mark/space pairs) + 1 (stop mark) = 67.
pub const FRAME_PULSES: usize = 2 + DATA_BITS * 2 + 1;

/// Check whether `actual` is within [`TOLERANCE_US`] of `expected`.
const fn is_close(actual: u32, expected: u32) -> bool {
    let lo = expected.saturating_sub(TOLERANCE_US);
    let hi = expected.saturating_add(TOLERANCE_US);
    actual >= lo && actual <= hi
}

/// Try to decode a NEC frame from the given pulse slice.
///
/// Returns `Some(NecCommand)` on a well-formed 32-bit frame with a
/// valid `cmd` / `~cmd` checksum. Returns `None` if the slice is too
/// short, the preamble doesn't match, any bit-space is out of range,
/// or the command checksum fails.
///
/// The slice can be longer than one full frame (67 pulses); the
/// decoder only looks at the first 67 entries and silently ignores
/// trailing pulses. That keeps callers free from having to trim
/// captures.
///
/// Note: does **not** require the final stop mark's timing to be
/// close to the 560 µs bit-mark — some receivers clip the trailing
/// mark short, and a missing stop pulse shouldn't discard an
/// otherwise valid frame.
#[must_use]
pub fn decode(pulses: &[Pulse]) -> Option<NecCommand> {
    if pulses.len() < FRAME_PULSES {
        return None;
    }
    // Preamble: mark (IR on) then space (IR off).
    if !pulses[0].level || !is_close(pulses[0].duration_us, PREAMBLE_MARK_US) {
        return None;
    }
    if pulses[1].level || !is_close(pulses[1].duration_us, PREAMBLE_SPACE_US) {
        return None;
    }

    // 32 data bits as (mark, space) pairs starting at index 2.
    let mut bits: u32 = 0;
    for bit_index in 0..DATA_BITS {
        let mark_idx = 2 + bit_index * 2;
        let space_idx = mark_idx + 1;
        let mark = pulses[mark_idx];
        let space = pulses[space_idx];
        if !mark.level || !is_close(mark.duration_us, BIT_MARK_US) {
            return None;
        }
        if space.level {
            return None;
        }
        // Decide bit value by space length.
        let bit = if is_close(space.duration_us, BIT_ZERO_SPACE_US) {
            0
        } else if is_close(space.duration_us, BIT_ONE_SPACE_US) {
            1
        } else {
            return None;
        };
        // NEC transmits LSB-first. Place each bit at its final
        // receive position so the complete u32 has byte layout:
        // `[addr_low, addr_high, cmd, ~cmd]` in little-endian order.
        bits |= bit << bit_index;
    }

    // Split into the 4 bytes and validate the command checksum.
    let addr_low = (bits & 0xFF) as u8;
    let addr_high = ((bits >> 8) & 0xFF) as u8;
    let command = ((bits >> 16) & 0xFF) as u8;
    let command_inv = ((bits >> 24) & 0xFF) as u8;
    if command ^ command_inv != 0xFF {
        return None;
    }

    Some(NecCommand {
        address: u16::from(addr_low) | (u16::from(addr_high) << 8),
        command,
    })
}

impl NecCommand {
    /// Encode the command as a full 67-pulse NEC frame ready to hand to
    /// an RMT TX peripheral.
    ///
    /// Preamble (9 ms mark + 4.5 ms space), 32 data bits
    /// (`addr_low, addr_high, command, ~command`, each LSB-first), and
    /// a final 560 µs stop mark. `level = true` means "IR carrier on"
    /// (mark); callers driving active-low TX hardware should invert
    /// before emission.
    ///
    /// The `~command` byte is computed from [`Self::command`] so senders
    /// and receivers agree on the checksum byte without the caller
    /// having to supply it.
    #[must_use]
    pub fn encode(&self) -> [Pulse; FRAME_PULSES] {
        let command_inv = !self.command;
        #[allow(
            clippy::cast_possible_truncation,
            reason = "masked by 0xFF / shifted right before cast"
        )]
        let bytes: [u8; 4] = [
            (self.address & 0xFF) as u8,
            ((self.address >> 8) & 0xFF) as u8,
            self.command,
            command_inv,
        ];
        let mut pulses = [Pulse {
            level: false,
            duration_us: 0,
        }; FRAME_PULSES];
        pulses[0] = Pulse {
            level: true,
            duration_us: PREAMBLE_MARK_US,
        };
        pulses[1] = Pulse {
            level: false,
            duration_us: PREAMBLE_SPACE_US,
        };
        let mut i = 2;
        for byte in bytes {
            for bit in 0..8 {
                pulses[i] = Pulse {
                    level: true,
                    duration_us: BIT_MARK_US,
                };
                let one = (byte >> bit) & 1 != 0;
                pulses[i + 1] = Pulse {
                    level: false,
                    duration_us: if one {
                        BIT_ONE_SPACE_US
                    } else {
                        BIT_ZERO_SPACE_US
                    },
                };
                i += 2;
            }
        }
        // Stop bit: short mark.
        pulses[i] = Pulse {
            level: true,
            duration_us: BIT_MARK_US,
        };
        pulses
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a well-formed NEC frame via [`NecCommand::encode`] for
    /// tests that verify the decoder.
    fn build_frame(address: u16, command: u8) -> [Pulse; FRAME_PULSES] {
        NecCommand { address, command }.encode()
    }

    #[test]
    fn well_formed_frame_round_trips() {
        let frame = build_frame(0xABCD, 0x42);
        let Some(cmd) = decode(&frame) else {
            unreachable!("valid frame")
        };
        assert_eq!(cmd.address, 0xABCD);
        assert_eq!(cmd.command, 0x42);
    }

    #[test]
    fn inverted_checksum_rejected() {
        // command = 0x00 → ~command = 0xFF, bit 0 of ~command = 1
        // (long space). Flip bit 0 of the *command* byte (which is 0,
        // short space) to 1 (long space). After the flip the
        // command / ~command XOR no longer equals 0xFF.
        let mut frame = build_frame(0x0000, 0x00);
        // Command byte starts at pulse index 2 + 16 * 2 = 34. Bit 0's
        // space lives at `mark_idx + 1 = 35`.
        let space_of_command_bit0 = 35;
        frame[space_of_command_bit0].duration_us = BIT_ONE_SPACE_US;
        assert!(decode(&frame).is_none());
    }

    #[test]
    fn preamble_out_of_tolerance_rejected() {
        let mut frame = build_frame(0x0000, 0x00);
        frame[0].duration_us = 2_000; // way off 9 ms
        assert!(decode(&frame).is_none());
    }

    #[test]
    fn too_short_buffer_rejected() {
        let short = [Pulse {
            level: true,
            duration_us: PREAMBLE_MARK_US,
        }; 10];
        assert!(decode(&short).is_none());
    }

    #[test]
    fn tolerance_absorbs_small_jitter() {
        let mut frame = build_frame(0x00FF, 0xA5);
        // Wiggle every pulse within tolerance.
        for p in &mut frame {
            p.duration_us = p.duration_us.saturating_add(150);
        }
        let Some(cmd) = decode(&frame) else {
            unreachable!("within tolerance")
        };
        assert_eq!(cmd.address, 0x00FF);
        assert_eq!(cmd.command, 0xA5);
    }

    #[test]
    fn encode_then_decode_round_trips() {
        let original = NecCommand {
            address: 0xBEEF,
            command: 0x5A,
        };
        let frame = original.encode();
        let Some(decoded) = decode(&frame) else {
            unreachable!("encoded frame must decode")
        };
        assert_eq!(decoded, original);
    }

    #[test]
    fn encoded_frame_has_correct_preamble() {
        let frame = NecCommand {
            address: 0,
            command: 0,
        }
        .encode();
        assert!(frame[0].level);
        assert_eq!(frame[0].duration_us, PREAMBLE_MARK_US);
        assert!(!frame[1].level);
        assert_eq!(frame[1].duration_us, PREAMBLE_SPACE_US);
        // Stop mark at the end.
        assert!(frame[FRAME_PULSES - 1].level);
        assert_eq!(frame[FRAME_PULSES - 1].duration_us, BIT_MARK_US);
    }

    #[test]
    fn trailing_pulses_ignored() {
        let frame = build_frame(0x1234, 0x56);
        let mut padded = [Pulse {
            level: false,
            duration_us: 0,
        }; FRAME_PULSES + 20];
        padded[..FRAME_PULSES].copy_from_slice(&frame);
        let Some(cmd) = decode(&padded) else {
            unreachable!("trailing garbage ignored")
        };
        assert_eq!(cmd.command, 0x56);
    }
}
