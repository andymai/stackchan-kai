//! Pull-based PCM audio source trait.
//!
//! [`AudioSource`] is the queue-element type the firmware audio task
//! consumes during TX playback. Implementors include short SFX
//! generators (sine-cycle tables looped N times), baked-PCM clips
//! (`include_bytes!`-embedded i16 slices), and streamed sources
//! (cloud TTS chunks arriving over a channel).
//!
//! ## Sample format
//!
//! 16-bit signed mono at the firmware's I²S sample rate (16 kHz on the
//! CoreS3). Sources that produce other formats convert at the boundary
//! — keeps the consumer side uniform.
//!
//! ## Pull, not push
//!
//! `fill(buf)` matches how the existing `i2s_tx.push_with` closure pulls
//! samples from the queue: the DMA layer owns the buffer, the source
//! writes into it. Bulk-copy is more efficient than the
//! sample-at-a-time `next_sample()` shape it replaces.

use stackchan_core::lipsync::LipSync;

/// Pull-based 16-bit signed mono PCM source at the audio task's
/// configured sample rate.
///
/// Implementations are owned (boxed via `Box<dyn AudioSource>`) and
/// consumed by the firmware speech router's TX feeder. A source is
/// "exhausted" when [`Self::fill`] returns `0`; the feeder drops it
/// and pulls the next queued source.
pub trait AudioSource: Send {
    /// Write up to `buf.len()` samples into `buf` and return the number
    /// of samples actually written.
    ///
    /// Contract:
    /// - `0` means the source is exhausted; the caller drops it.
    /// - A value less than `buf.len()` is allowed for the final chunk;
    ///   the caller fills the remainder with silence.
    /// - Streaming sources may return a partial fill while waiting for
    ///   data; the caller treats that as "play silence this batch and
    ///   try again next tick" rather than as exhaustion.
    fn fill(&mut self, buf: &mut [i16]) -> usize;

    /// Lip-sync hint corresponding to the most recent fill, if the
    /// source can produce one.
    ///
    /// `None` means "no native data; consumer falls back to live RMS
    /// on outgoing samples." Backends that ship a baked envelope
    /// timeline or receive alignment metadata from a cloud API
    /// override this.
    #[must_use]
    fn lip_sync(&self) -> Option<LipSync> {
        None
    }

    /// Approximate samples remaining, if known.
    ///
    /// Used by the router to decide whether a queued lower-priority
    /// source should preempt the current one (long silences are good
    /// preemption points). `None` for streaming / unknown-length
    /// sources.
    #[must_use]
    fn len_hint(&self) -> Option<usize> {
        None
    }
}

#[cfg(test)]
#[allow(clippy::missing_docs_in_private_items)]
mod tests {
    use super::*;
    use alloc::boxed::Box;

    /// Trivial source that yields a fixed number of zero samples then
    /// exhausts. Pins the trait contract: fill returns sample count;
    /// 0 means done.
    struct ZeroSource {
        remaining: usize,
    }

    impl AudioSource for ZeroSource {
        fn fill(&mut self, buf: &mut [i16]) -> usize {
            let n = buf.len().min(self.remaining);
            buf[..n].fill(0);
            self.remaining -= n;
            n
        }

        fn len_hint(&self) -> Option<usize> {
            Some(self.remaining)
        }
    }

    #[test]
    fn fill_yields_then_exhausts() {
        let mut src: Box<dyn AudioSource> = Box::new(ZeroSource { remaining: 5 });
        let mut buf = [99_i16; 8];
        let n = src.fill(&mut buf);
        assert_eq!(n, 5);
        assert_eq!(&buf[..5], &[0, 0, 0, 0, 0]);
        // Trailing slots untouched — caller fills with silence.
        assert_eq!(buf[5], 99);
        // Next call: exhausted.
        let n = src.fill(&mut buf);
        assert_eq!(n, 0);
    }

    #[test]
    fn lip_sync_defaults_to_none() {
        let src = ZeroSource { remaining: 0 };
        assert!(src.lip_sync().is_none());
    }

    #[test]
    fn len_hint_round_trips() {
        let src = ZeroSource { remaining: 42 };
        assert_eq!(src.len_hint(), Some(42));
    }
}
