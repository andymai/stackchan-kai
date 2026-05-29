from __future__ import annotations

from pathlib import Path

from lens_calibration.cli.lens import main


def _capture(nx_series: list[float], ny_series: list[float]) -> str:
    rows = []
    for nx, ny in zip(nx_series, ny_series, strict=True):
        cx = int(nx * 1000)
        cy = int(ny * 1000)
        pan = int(nx * 10 * 100)
        tilt = int(-ny * 10 * 100)
        rows.append(
            f"1000 ms tracker-bench: motion=Tracking fired=12 "
            f"centroid=({cx}/1000, {cy}/1000) target_pose=({pan}/100°, {tilt}/100°)"
        )
    return "\n".join(rows) + "\n"


CONVERGING = [0.6, 0.45, 0.32, 0.21, 0.12, 0.05]
DIVERGING = [0.1, 0.2, 0.33, 0.48, 0.62, 0.75]


def test_correct_mount_exit_zero(tmp_path: Path) -> None:
    cap = tmp_path / "good.log"
    cap.write_text(_capture(CONVERGING, CONVERGING), encoding="utf-8")
    assert main(["--input", str(cap)]) == 0


def test_flipped_axis_exit_one(tmp_path: Path) -> None:
    cap = tmp_path / "flipped.log"
    cap.write_text(_capture(DIVERGING, CONVERGING), encoding="utf-8")
    assert main(["--input", str(cap)]) == 1


def test_no_frames_exit_two(tmp_path: Path) -> None:
    cap = tmp_path / "empty.log"
    cap.write_text("1000 ms INFO unrelated subsystem chatter\n", encoding="utf-8")
    assert main(["--input", str(cap)]) == 2


def test_missing_input_exit_two(tmp_path: Path) -> None:
    assert main(["--input", str(tmp_path / "nope.log")]) == 2


def test_stdin_path(monkeypatch, capsys) -> None:  # type: ignore[no-untyped-def]
    import io

    monkeypatch.setattr("sys.stdin", io.StringIO(_capture(CONVERGING, CONVERGING)))
    assert main([]) == 0
    out = capsys.readouterr().out
    assert "PASS" in out
