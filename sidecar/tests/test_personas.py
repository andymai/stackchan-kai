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


def test_load_persona_rejects_empty_name(tmp_path: Path) -> None:
    with pytest.raises(ValueError, match="non-empty"):
        load_persona("", tmp_path)


def test_load_persona_rejects_path_traversal(tmp_path: Path) -> None:
    # Real attack surface: an X-Persona-Name header value could
    # otherwise resolve outside the personas dir and read arbitrary
    # files on the sidecar host.
    for bad in ["../etc", "..\\etc", "foo/bar", "foo\\bar", "..foo", "foo..bar"]:
        with pytest.raises(ValueError):
            load_persona(bad, tmp_path)


def test_load_persona_rejects_control_chars(tmp_path: Path) -> None:
    # Header values shouldn't reach load_persona with controls (the
    # HTTP layer rejects them earlier), but defence in depth.
    for bad in ["foo\r\nX-Inject: yes", "foo\tbar", "foo\x00bar"]:
        with pytest.raises(ValueError):
            load_persona(bad, tmp_path)


def test_load_persona_rejects_oversize_name(tmp_path: Path) -> None:
    with pytest.raises(ValueError, match="exceeds"):
        load_persona("a" * 65, tmp_path)


def test_load_persona_accepts_max_length(tmp_path: Path) -> None:
    name = "a" * 64
    (tmp_path / f"{name}.md").write_text("Hi.", encoding="utf-8")
    assert load_persona(name, tmp_path) == "Hi."
