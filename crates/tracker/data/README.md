# Cascade weights

Frontal-face Haar cascades sourced from OpenCV's reference XML
distribution and converted to Rust `const` arrays by the workspace
`xtask-cascade-convert` binary.

| File                                       | Source                                                                                          | License                                                            |
| ------------------------------------------ | ----------------------------------------------------------------------------------------------- | ------------------------------------------------------------------ |
| `haarcascade_frontalface_default.xml`      | <https://github.com/opencv/opencv/blob/4.x/data/haarcascades/haarcascade_frontalface_default.xml> | Intel Open Source Computer Vision Library license (BSD-3-Clause-style) — see XML header |

The original Intel/OpenCV license text is preserved verbatim in the XML
header. Treat redistribution and attribution requirements as binding on
the generated `crates/tracker/src/cascade_data.rs` as well as the XML.

## Regenerating the cascade module

```bash
cargo run --package xtask-cascade-convert --release \
    -- crates/tracker/data/haarcascade_frontalface_default.xml \
       crates/tracker/src/cascade_data.rs
cargo fmt --package tracker
```

The generated file is checked in so the `tracker` crate has zero
build-time dependencies on the converter.
