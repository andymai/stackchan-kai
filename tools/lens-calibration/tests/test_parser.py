from __future__ import annotations

from lens_calibration.parser import parse_tracker_lines

# Verbatim `log_outcome` format from
# crates/stackchan-firmware/examples/tracker_bench.rs, with a defmt
# `<ms> ms` timestamp prefix on some rows and not others, plus the
# startup banner and camera notice the bench also emits.
CAPTURE = [
    "1200 ms INFO tracker-bench v0.1.0 — CoreS3 boot, will stream GC0308 frames into the tracker",
    "1210 ms INFO tracker-bench: camera mode forced ON; consuming frames",
    "1300 ms tracker-bench: motion=Warmup fired=0 "
    "centroid=(0/1000, 0/1000) target_pose=(0/100°, 0/100°)",
    "1400 ms tracker-bench: motion=Tracking fired=12 "
    "centroid=(420/1000, -310/1000) target_pose=(240/100°, 160/100°)",
    "tracker-bench: motion=Tracking fired=8 "
    "centroid=(-150/1000, 90/1000) target_pose=(-90/100°, -40/100°)",
    "1500 ms tracker-bench: motion=GlobalEvent fired=40 "
    "centroid=(0/1000, 0/1000) target_pose=(0/100°, 0/100°)",
]


def test_parses_decision_rows_with_and_without_prefix() -> None:
    result = parse_tracker_lines(CAPTURE)
    motions = [f.motion for f in result.frames]
    assert motions == ["Warmup", "Tracking", "Tracking", "GlobalEvent"]
    assert result.unparsed == 0


def test_centroid_and_pose_unscaled() -> None:
    result = parse_tracker_lines(CAPTURE)
    tracking = next(f for f in result.frames if f.motion == "Tracking")
    assert tracking.fired == 12
    assert tracking.nx == 0.42
    assert tracking.ny == -0.31
    assert tracking.pan_deg == 2.40
    assert tracking.tilt_deg == 1.60


def test_banner_and_camera_notice_not_counted_unparsed() -> None:
    result = parse_tracker_lines(
        [
            "tracker-bench v0.1.0 — CoreS3 boot, will stream frames",
            "tracker-bench: camera mode forced ON; consuming frames",
        ]
    )
    assert result.frames == []
    assert result.unparsed == 0


def test_truncated_decision_row_counted_unparsed_not_raised() -> None:
    result = parse_tracker_lines(["tracker-bench: motion=Tracking fired=12 centroid=(420/1000,"])
    assert result.frames == []
    assert result.unparsed == 1


def test_degree_sign_optional_in_pose() -> None:
    # Tolerate captures whose pose fields drop the ° glyph.
    result = parse_tracker_lines(
        [
            "tracker-bench: motion=Tracking fired=3 centroid=(100/1000, 200/1000) "
            "target_pose=(50/100, -75/100)"
        ]
    )
    assert len(result.frames) == 1
    assert result.frames[0].pan_deg == 0.5
    assert result.frames[0].tilt_deg == -0.75


def test_non_tracker_lines_ignored() -> None:
    result = parse_tracker_lines(["1000 ms INFO some other subsystem log", ""])
    assert result.frames == []
    assert result.unparsed == 0
