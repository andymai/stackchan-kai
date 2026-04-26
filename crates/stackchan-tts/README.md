# stackchan-tts

Speech synthesis abstractions for the Stack-chan.

Defines the trait surfaces the firmware speech path runs against:

- [`AudioSource`] — pull-based PCM stream (sine cycle, baked clip, network chunk).
- [`SpeechBackend`] — turns an [`Utterance`] into an `AudioSource`.
- [`LipSync`] — envelope + optional viseme tag, published alongside playback.

Domain types (`Utterance`, `PhraseId`, `Locale`, `SpeechContent`, `SpeechStyle`, `Priority`, `ContentRef`) live in `stackchan-core::voice`; this crate is the implementation layer.

`no_std` with unconditional `alloc` — `Box<dyn AudioSource>` is the queue element type.

[`AudioSource`]: src/source.rs
[`SpeechBackend`]: src/backend.rs
[`LipSync`]: src/lipsync.rs
[`Utterance`]: ../stackchan-core/src/voice.rs
