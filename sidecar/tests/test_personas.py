from pathlib import Path

import pytest

from stackchan_sidecar.personas import load_persona


def test_load_persona_strips_frontmatter(tmp_path: Path) -> None:
    p = tmp_path / "alpha.md"
    p.write_text("---\nname: alpha\nvoice: warm\n---\nHello world.\n", encoding="utf-8")
    assert load_persona("alpha", tmp_path) == "Hello world."


def test_load_persona_without_frontmatter(tmp_path: Path) -> None:
    p = tmp_path / "beta.md"
    p.write_text("Just a prompt.\n", encoding="utf-8")
    assert load_persona("beta", tmp_path) == "Just a prompt."


def test_load_persona_missing_raises(tmp_path: Path) -> None:
    with pytest.raises(FileNotFoundError):
        load_persona("nope", tmp_path)


def test_load_persona_handles_unterminated_frontmatter(tmp_path: Path) -> None:
    p = tmp_path / "weird.md"
    p.write_text("---\nname: weird\nno closing\nstill no closing\n", encoding="utf-8")
    result = load_persona("weird", tmp_path)
    assert result.startswith("---")
