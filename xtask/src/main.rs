//! Convert `OpenCV` Haar cascade XML into the `tracker::cascade` Rust
//! `const` data layout.
//!
//! Reads `haarcascade_frontalface_default.xml` (or any compatible
//! frontal Haar cascade) and emits a single `cascade_data.rs` with one
//! `STAGE_n_STUMPS` array per stage plus a top-level `STAGES` array
//! and a `pub const FRONTAL_FACE: Cascade` referencing them.
//!
//! The generated file lives at `crates/tracker/src/cascade_data.rs` and
//! is checked into the repo. The converter is offline tooling — the
//! `tracker` crate at runtime depends only on the generated module.
//!
//! ## Limitations
//!
//! The converter rejects cascades that use:
//! * Tilted (45°-rotated) features. The runtime scorer only handles
//!   axis-aligned features.
//! * Non-stump weak classifiers (depth > 1). Every published frontal
//!   cascade is stump-based; deeper trees would need a different
//!   runtime layout.
//! * Non-integer rectangle weights. `OpenCV` stores weights as floats
//!   but every published frontal cascade uses small integers
//!   (`-1`, `+1`, `+2`, `+3`); we keep them as `i8` for compactness.

use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use roxmltree::Document;

/// Compact representation of one cascade rectangle.
struct Rect {
    /// Top-left x in base-window units.
    x: u8,
    /// Top-left y in base-window units.
    y: u8,
    /// Width in base-window units.
    w: u8,
    /// Height in base-window units.
    h: u8,
    /// Per-rectangle weight (always a small integer in published cascades).
    weight: i8,
}

/// One Haar feature: 2 or 3 rectangles.
struct Feature {
    /// Rectangles making up this feature; 2 or 3 entries.
    rects: Vec<Rect>,
}

/// One stump-based weak classifier.
struct Stump {
    /// Index into [`Cascade::features`] for the discriminating feature.
    feature_idx: usize,
    /// Comparison threshold in stddev-fraction units.
    threshold: f32,
    /// Output value when feature value is below `threshold`.
    left_val: f32,
    /// Output value when feature value is at or above `threshold`.
    right_val: f32,
}

/// One cascade stage.
struct Stage {
    /// Cumulative-score threshold for this stage.
    threshold: f32,
    /// Weak classifiers (stumps) belonging to this stage.
    stumps: Vec<Stump>,
}

/// Parsed cascade: window dims, feature pool, stages.
struct Cascade {
    /// Base window width in pixels.
    window_w: u8,
    /// Base window height in pixels.
    window_h: u8,
    /// Flat feature pool — stumps reference these by index.
    features: Vec<Feature>,
    /// Ordered stages.
    stages: Vec<Stage>,
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    if args.len() != 3 {
        eprintln!(
            "usage: {} <input.xml> <output.rs>",
            args.first().map_or("xtask-cascade-convert", String::as_str)
        );
        return ExitCode::from(2);
    }
    let in_path = PathBuf::from(&args[1]);
    let out_path = PathBuf::from(&args[2]);

    let xml = match fs::read_to_string(&in_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: failed to read {}: {e}", in_path.display());
            return ExitCode::from(1);
        }
    };
    let doc = match Document::parse(&xml) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: invalid XML: {e}");
            return ExitCode::from(1);
        }
    };

    let cascade = match parse_cascade(&doc) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: cascade parse failed: {e}");
            return ExitCode::from(1);
        }
    };

    let rendered = render_cascade(&cascade, &in_path);
    if let Err(e) = fs::write(&out_path, rendered) {
        eprintln!("error: failed to write {}: {e}", out_path.display());
        return ExitCode::from(1);
    }

    eprintln!(
        "ok: {} stages, {} stumps, {} features → {}",
        cascade.stages.len(),
        cascade.stages.iter().map(|s| s.stumps.len()).sum::<usize>(),
        cascade.features.len(),
        out_path.display(),
    );
    ExitCode::SUCCESS
}

/// Top-level XML walker: `opencv_storage` / `cascade`.
#[allow(
    clippy::too_many_lines,
    reason = "single linear walk of a flat XML schema (cascade → \
              features → stages → stumps); splitting helpers obscures \
              the field-by-field correspondence with the source XML"
)]
fn parse_cascade(doc: &Document<'_>) -> Result<Cascade, String> {
    let root = doc
        .descendants()
        .find(|n| n.has_tag_name("cascade"))
        .ok_or_else(|| "missing <cascade> element".to_string())?;

    let feature_type = child_text(root, "featureType")?;
    if feature_type != "HAAR" {
        return Err(format!("unsupported featureType: {feature_type}"));
    }
    let stage_type = child_text(root, "stageType")?;
    if stage_type != "BOOST" {
        return Err(format!("unsupported stageType: {stage_type}"));
    }
    let window_w: u8 = child_text(root, "width")?
        .parse()
        .map_err(|e| format!("width: {e}"))?;
    let window_h: u8 = child_text(root, "height")?
        .parse()
        .map_err(|e| format!("height: {e}"))?;

    let features_node = root
        .children()
        .find(|n| n.has_tag_name("features"))
        .ok_or_else(|| "missing <features> element".to_string())?;
    let mut features = Vec::new();
    for feat_node in features_node.children().filter(|n| n.has_tag_name("_")) {
        // Reject tilted features outright — runtime scorer is
        // axis-aligned only.
        if let Some(t) = feat_node.children().find(|n| n.has_tag_name("tilted")) {
            let v: i32 = t.text().unwrap_or("0").trim().parse().unwrap_or(0);
            if v != 0 {
                return Err("tilted features are not supported by this runtime".to_string());
            }
        }
        let rects_node = feat_node
            .children()
            .find(|n| n.has_tag_name("rects"))
            .ok_or_else(|| "feature has no <rects>".to_string())?;
        let mut rects = Vec::new();
        for r_node in rects_node.children().filter(|n| n.has_tag_name("_")) {
            let raw = r_node.text().unwrap_or("").trim();
            // Format: "x y w h weight"
            let parts: Vec<&str> = raw.split_whitespace().collect();
            if parts.len() < 5 {
                return Err(format!("rect has fewer than 5 fields: {raw:?}"));
            }
            let x: u8 = parts[0].parse().map_err(|e| format!("rect x: {e}"))?;
            let y: u8 = parts[1].parse().map_err(|e| format!("rect y: {e}"))?;
            let w: u8 = parts[2].parse().map_err(|e| format!("rect w: {e}"))?;
            let h: u8 = parts[3].parse().map_err(|e| format!("rect h: {e}"))?;
            let weight_raw: f64 = parts[4]
                .trim_end_matches('.')
                .parse()
                .map_err(|e| format!("rect weight: {e}"))?;
            // Weights are always small integers in published cascades.
            // Reject anything fractional so the i8 packing is sound.
            if weight_raw.fract() != 0.0 {
                return Err(format!("non-integer rect weight: {weight_raw}"));
            }
            if !(-128.0..=127.0).contains(&weight_raw) {
                return Err(format!("rect weight {weight_raw} out of i8 range"));
            }
            #[allow(clippy::cast_possible_truncation)]
            let weight = weight_raw as i8;
            rects.push(Rect { x, y, w, h, weight });
        }
        if rects.len() < 2 || rects.len() > 3 {
            return Err(format!(
                "feature has {} rects (expected 2 or 3)",
                rects.len()
            ));
        }
        features.push(Feature { rects });
    }

    let stages_node = root
        .children()
        .find(|n| n.has_tag_name("stages"))
        .ok_or_else(|| "missing <stages> element".to_string())?;
    let mut stages = Vec::new();
    for stage_node in stages_node.children().filter(|n| n.has_tag_name("_")) {
        let stage_threshold: f32 = child_text(stage_node, "stageThreshold")?
            .parse()
            .map_err(|e| format!("stageThreshold: {e}"))?;
        let weak_node = stage_node
            .children()
            .find(|n| n.has_tag_name("weakClassifiers"))
            .ok_or_else(|| "missing <weakClassifiers>".to_string())?;
        let mut stumps = Vec::new();
        for wc_node in weak_node.children().filter(|n| n.has_tag_name("_")) {
            let internal = child_text(wc_node, "internalNodes")?;
            let leaves = child_text(wc_node, "leafValues")?;
            // internalNodes for a stump: "<leftFlag> <rightFlag> <featIdx> <threshold>".
            // The flag fields encode tree topology; for stumps both
            // children are leaves so we skip them. Anything other than
            // a single 4-field line is a non-stump tree we reject.
            let parts: Vec<&str> = internal.split_whitespace().collect();
            if parts.len() != 4 {
                return Err(format!(
                    "weak classifier has {} internalNodes fields (expected 4 for stump)",
                    parts.len()
                ));
            }
            let feature_idx: usize = parts[2]
                .parse()
                .map_err(|e| format!("internalNodes feature_idx: {e}"))?;
            let threshold: f32 = parts[3]
                .parse()
                .map_err(|e| format!("internalNodes threshold: {e}"))?;
            let leaf_parts: Vec<&str> = leaves.split_whitespace().collect();
            if leaf_parts.len() != 2 {
                return Err(format!(
                    "weak classifier has {} leafValues (expected 2 for stump)",
                    leaf_parts.len()
                ));
            }
            let left_val: f32 = leaf_parts[0]
                .parse()
                .map_err(|e| format!("leafValues[0]: {e}"))?;
            let right_val: f32 = leaf_parts[1]
                .parse()
                .map_err(|e| format!("leafValues[1]: {e}"))?;
            stumps.push(Stump {
                feature_idx,
                threshold,
                left_val,
                right_val,
            });
        }
        stages.push(Stage {
            threshold: stage_threshold,
            stumps,
        });
    }

    Ok(Cascade {
        window_w,
        window_h,
        features,
        stages,
    })
}

/// Read the `<tag>...</tag>` text directly under `parent` and return it
/// trimmed.
fn child_text<'a>(parent: roxmltree::Node<'a, '_>, tag: &str) -> Result<&'a str, String> {
    parent
        .children()
        .find(|n| n.has_tag_name(tag))
        .and_then(|n| n.text())
        .map(str::trim)
        .ok_or_else(|| format!("missing <{tag}>"))
}

/// Render the parsed cascade into a self-contained Rust source file.
fn render_cascade(c: &Cascade, source: &Path) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "//! Cascade weights for the `tracker::cascade` scorer.\n\
         //!\n\
         //! Generated by `xtask-cascade-convert` from `{}`.\n\
         //! Do not edit by hand — re-run the converter to regenerate.\n\
         //!\n\
         //! Source license: see `crates/tracker/data/README.md`.",
        source
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("<input>"),
    );
    // Generated tables: per-stage stump arrays are intentionally
    // undocumented (their content is fully described by this module's
    // crate-level docs and the source XML).
    let _ = writeln!(
        out,
        "#![allow(\n    \
            clippy::unreadable_literal,\n    \
            clippy::excessive_precision,\n    \
            clippy::approx_constant,\n    \
            clippy::neg_multiply,\n    \
            clippy::missing_docs_in_private_items,\n    \
            missing_docs\n\
         )]"
    );
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "use crate::cascade::{{Cascade, Feature, Rect, Stage, Stump, MAX_RECTS_PER_FEATURE}};"
    );
    let _ = writeln!(out);

    // Per-stage stump arrays. Each stump references its feature
    // inline rather than going through a feature-index table — keeps
    // the runtime cache-friendly and lets the scorer skip the extra
    // indirection.
    for (stage_idx, stage) in c.stages.iter().enumerate() {
        let _ = writeln!(
            out,
            "const STAGE_{stage_idx}_STUMPS: [Stump; {len}] = [",
            len = stage.stumps.len()
        );
        for stump in &stage.stumps {
            let feat = &c.features[stump.feature_idx];
            let _ = writeln!(out, "    Stump {{");
            let _ = writeln!(out, "        feature: Feature {{");
            let _ = writeln!(out, "            rects: [");
            for slot in 0..3 {
                if slot < feat.rects.len() {
                    let r = &feat.rects[slot];
                    let _ = writeln!(
                        out,
                        "                Rect {{ x: {}, y: {}, w: {}, h: {}, weight: {} }},",
                        r.x, r.y, r.w, r.h, r.weight,
                    );
                } else {
                    let _ = writeln!(
                        out,
                        "                Rect {{ x: 0, y: 0, w: 0, h: 0, weight: 0 }},"
                    );
                }
            }
            let _ = writeln!(out, "            ],");
            let _ = writeln!(out, "            rect_count: {},", feat.rects.len());
            let _ = writeln!(out, "        }},");
            let _ = writeln!(out, "        threshold: {},", format_f32(stump.threshold));
            let _ = writeln!(out, "        left_val: {},", format_f32(stump.left_val));
            let _ = writeln!(out, "        right_val: {},", format_f32(stump.right_val));
            let _ = writeln!(out, "    }},");
        }
        let _ = writeln!(out, "];");
        let _ = writeln!(out);
    }

    // STAGES array referencing the per-stage stumps.
    let _ = writeln!(out, "const STAGES: [Stage; {}] = [", c.stages.len());
    for (idx, stage) in c.stages.iter().enumerate() {
        let _ = writeln!(
            out,
            "    Stage {{ stumps: &STAGE_{idx}_STUMPS, threshold: {} }},",
            format_f32(stage.threshold),
        );
    }
    let _ = writeln!(out, "];");
    let _ = writeln!(out);

    // Top-level cascade.
    let _ = writeln!(
        out,
        "/// Frontal-face cascade compiled from `OpenCV`'s reference XML.\n\
         pub const FRONTAL_FACE: Cascade = Cascade {{\n    \
            window_w: {w},\n    \
            window_h: {h},\n    \
            stages: &STAGES,\n\
         }};",
        w = c.window_w,
        h = c.window_h,
    );

    // Compile-time assertion that `MAX_RECTS_PER_FEATURE` is sufficient.
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "const _: () = assert!(MAX_RECTS_PER_FEATURE >= 3, \"feature rect array must hold at least 3 rectangles\");"
    );

    out
}

/// Format an `f32` with enough precision to round-trip and an explicit
/// `_f32` suffix so rustfmt leaves it alone.
fn format_f32(v: f32) -> String {
    // {:e} keeps every significant digit and is unambiguous for both
    // very small and very large values; tag with the suffix so it's
    // typed regardless of context.
    if v == 0.0 {
        "0.0_f32".to_string()
    } else {
        format!("{v:e}_f32")
    }
}
