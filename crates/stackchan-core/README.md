# stackchan-core

`no_std`, dependency-free domain library for the StackChan avatar. Models
the face as data (`Avatar` holding two eyes + a mouth + an emotion) and
drives animation through a `Modifier` trait that mutates the avatar in
response to time.

The crate has no hardware, OS, or allocation dependencies — it's the
platform-independent heart of the firmware. A renderer (in
`stackchan-firmware`) turns `Avatar` into pixels; a simulator (in
`stackchan-sim`) drives `Modifier`s against a fake clock for testing.

See the workspace [README](../../README.md) and [CLAUDE.md](../../CLAUDE.md)
for project-wide conventions.
