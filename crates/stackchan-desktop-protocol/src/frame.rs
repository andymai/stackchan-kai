//! Newline-delimited byte accumulator.
//!
//! BLE notifications fragment at the negotiated MTU boundary —
//! a 200-byte snapshot can arrive as one or two writes depending
//! on the link. [`LineFramer`] absorbs raw byte slices and yields
//! complete lines (`\n`-terminated, trailing `\r` stripped) one at
//! a time so the parser sees whole messages.

use alloc::vec::Vec;

/// Maximum line length the framer will buffer before dropping the
/// in-progress line.
///
/// The reference spec caps turn events at 4 KB and folder-push
/// chunks at a few KB. 8 KB gives headroom for a slightly larger
/// turn event without unbounded growth from a desktop bug or a
/// stuck partial write.
pub const MAX_LINE_BYTES: usize = 8192;

/// Incremental line accumulator for the NUS RX stream.
///
/// Push byte slices in any chunking; pop complete lines as they
/// arrive. The framer never allocates inside [`Self::push`] beyond
/// the rolling buffer; popped lines own their bytes.
#[derive(Debug, Default)]
pub struct LineFramer {
    /// In-progress line bytes, never containing a `\n`.
    buf: Vec<u8>,
    /// Number of bytes discarded because the in-progress line
    /// exceeded [`MAX_LINE_BYTES`]. Resets when a `\n` finally
    /// arrives and the framer recovers.
    overflow_discarded: usize,
}

impl LineFramer {
    /// Construct an empty framer.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            buf: Vec::new(),
            overflow_discarded: 0,
        }
    }

    /// Absorb a chunk of bytes and append any newly completed
    /// lines (CR-stripped) to `out`.
    ///
    /// Returns the number of complete lines pushed onto `out`.
    /// An overflowing line is silently discarded — recovery
    /// resumes at the next newline.
    pub fn push(&mut self, chunk: &[u8], out: &mut Vec<Vec<u8>>) -> usize {
        let mut pushed = 0;
        for &byte in chunk {
            if byte == b'\n' {
                if self.overflow_discarded == 0 {
                    let mut line = core::mem::take(&mut self.buf);
                    if line.last() == Some(&b'\r') {
                        line.pop();
                    }
                    if !line.is_empty() {
                        out.push(line);
                        pushed += 1;
                    }
                } else {
                    self.buf.clear();
                    self.overflow_discarded = 0;
                }
            } else if self.buf.len() >= MAX_LINE_BYTES {
                self.overflow_discarded += 1;
            } else {
                self.buf.push(byte);
            }
        }
        pushed
    }

    /// Drop any partially accumulated bytes. Call when the BLE
    /// link disconnects — any half-line is now meaningless.
    pub fn reset(&mut self) {
        self.buf.clear();
        self.overflow_discarded = 0;
    }

    /// Bytes currently buffered for the in-progress line.
    #[must_use]
    pub const fn pending_bytes(&self) -> usize {
        self.buf.len()
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "tests assert structural invariants; .expect / .unwrap are the standard test idiom"
)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn single_line_no_newline_yields_nothing() {
        let mut f = LineFramer::new();
        let mut out = Vec::new();
        assert_eq!(f.push(b"{\"total\":1}", &mut out), 0);
        assert!(out.is_empty());
        assert_eq!(f.pending_bytes(), 11);
    }

    #[test]
    fn single_complete_line() {
        let mut f = LineFramer::new();
        let mut out = Vec::new();
        assert_eq!(f.push(b"{\"x\":1}\n", &mut out), 1);
        assert_eq!(out, vec![b"{\"x\":1}".to_vec()]);
    }

    #[test]
    fn cr_lf_terminated() {
        let mut f = LineFramer::new();
        let mut out = Vec::new();
        f.push(b"{\"x\":1}\r\n", &mut out);
        assert_eq!(out, vec![b"{\"x\":1}".to_vec()]);
    }

    #[test]
    fn fragmented_across_pushes() {
        let mut f = LineFramer::new();
        let mut out = Vec::new();
        f.push(b"{\"to", &mut out);
        f.push(b"tal\":", &mut out);
        assert_eq!(f.push(b"1}\n", &mut out), 1);
        assert_eq!(out, vec![b"{\"total\":1}".to_vec()]);
    }

    #[test]
    fn two_lines_in_one_chunk() {
        let mut f = LineFramer::new();
        let mut out = Vec::new();
        assert_eq!(f.push(b"a\nb\n", &mut out), 2);
        assert_eq!(out, vec![b"a".to_vec(), b"b".to_vec()]);
    }

    #[test]
    fn empty_lines_dropped() {
        let mut f = LineFramer::new();
        let mut out = Vec::new();
        f.push(b"a\n\n\nb\n", &mut out);
        assert_eq!(out, vec![b"a".to_vec(), b"b".to_vec()]);
    }

    #[test]
    fn overflow_discarded_until_newline() {
        let mut f = LineFramer::new();
        let mut out = Vec::new();
        // Stuff in a runaway line ~2× the limit.
        let blob: Vec<u8> = (0..MAX_LINE_BYTES * 2)
            .map(|i| u8::try_from(i % 64).unwrap_or(0) + b'a')
            .collect();
        f.push(&blob, &mut out);
        assert!(out.is_empty());
        // The terminator triggers recovery (no line emitted) and
        // the next valid line works.
        f.push(b"\n{\"ok\":1}\n", &mut out);
        assert_eq!(out, vec![b"{\"ok\":1}".to_vec()]);
    }

    #[test]
    fn reset_drops_partial_line() {
        let mut f = LineFramer::new();
        let mut out = Vec::new();
        f.push(b"half", &mut out);
        f.reset();
        f.push(b"line\n", &mut out);
        assert_eq!(out, vec![b"line".to_vec()]);
    }
}
