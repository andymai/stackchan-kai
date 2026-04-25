# Stability Policy

This document defines what **stability** means for the crates in this
workspace. It is a promise about the rate at which APIs change, not a promise
that they never will.

> **Pre-v0.1.0 status:** no stability guarantees apply yet. This document
> describes the policy that begins at v0.1.0.

## Status levels

Each public item (module, type, function, feature flag) is classified as
**Stable**, **Experimental**, or **Internal**.

### Stable

A stable API has earned the following pledge:

- **Breaking changes ship only in planned major versions.** A major bump that
  touches a stable API is announced in a GitHub issue at least **60 days**
  before the release, along with a migration note in the CHANGELOG preamble.
- **Deprecation precedes removal.** A stable API scheduled for removal is
  marked `#[deprecated]` for **at least one major version** before it is
  actually deleted.
- **Semver is honored strictly.** No sneaking a breaking change into a minor
  or patch release.

### Experimental

An experimental API signals: *the shape is still being discovered*. Expect
breakage in any minor version, with no deprecation cycle. The API is real, not
a stub — fully implemented and tested — but its contract may shift.

Pin to an exact minor version in your `Cargo.toml` if depending on one:

```toml
stackchan-core = "=0.1"
```

Experimental APIs graduate to stable by explicit CHANGELOG entry.

### Internal

Internal items are `pub(crate)` or `#[doc(hidden)]`. Not part of the supported
surface.

## Day-one classification (v0.1.0 through v1.0)

At v0.1.0, **everything is experimental**. The initial release exists to prove
the flash + render pipeline end to end; the API surface will churn rapidly
through v0.2–v0.x as the avatar domain model settles.

v1.0 is a **polish milestone** — RON-configurable appearance, host-side
calibration tooling, crash recovery, codebase TODO sweep — and still ships
*everything Experimental*. Stabilization is intentionally deferred so the
API can survive the architectural work planned for v2.x.

Graduations to Stable will be called out in dedicated `feat(stabilize):`
commits, first appearing at **v2.x** or later.

## Wrapper crates

`stackchan-firmware` is a binary crate, not a library surface. Standalone
driver crates (`axp2101`, `aw9523`, `scservo`, `bm8563`, and others) may
publish to crates.io independently and are governed by this policy under
their own versioning. Their graduations from Experimental to Stable happen
on each crate's own cadence, not gated on the workspace version.

## Cadence commitment

Once any item graduates to Stable (planned for v2.x or later):

- **Stable surface**: at most one breaking change per 60 days.
- **Experimental surface**: no such bound.
- **Planned majors** will bundle breaking changes together.

This commitment applies going forward only; it is not retroactive.
