from pathlib import Path

import pytest

from stackchan_sidecar.personas import list_personas, load_persona


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


def test_list_personas_empty_when_dir_missing(tmp_path: Path) -> None:
    # Sidecar may run before the operator provisions personas; the
    # listing must not raise just because the dir isn't there yet.
    assert list_personas(tmp_path / "missing") == []


def test_list_personas_empty_when_dir_empty(tmp_path: Path) -> None:
    assert list_personas(tmp_path) == []


def test_list_personas_returns_slugs_sorted(tmp_path: Path) -> None:
    for name in ["zeta", "alpha", "beta"]:
        (tmp_path / f"{name}.md").write_text("Hi.", encoding="utf-8")
    assert list_personas(tmp_path) == ["alpha", "beta", "zeta"]


def test_list_personas_ignores_non_md_files(tmp_path: Path) -> None:
    (tmp_path / "stack-chan.md").write_text("Hi.", encoding="utf-8")
    (tmp_path / "README.txt").write_text("notes", encoding="utf-8")
    (tmp_path / "scratch.json").write_text("{}", encoding="utf-8")
    assert list_personas(tmp_path) == ["stack-chan"]


def test_list_personas_ignores_subdirectories(tmp_path: Path) -> None:
    (tmp_path / "stack-chan.md").write_text("Hi.", encoding="utf-8")
    (tmp_path / "archive").mkdir()
    (tmp_path / "archive" / "old.md").write_text("Old.", encoding="utf-8")
    assert list_personas(tmp_path) == ["stack-chan"]


def test_list_personas_skips_invalid_slugs(tmp_path: Path) -> None:
    # An operator dropping a `.md` whose stem can't be a slug
    # shouldn't break the listing — skip silently so the rest
    # remains discoverable.
    (tmp_path / "stack-chan.md").write_text("Hi.", encoding="utf-8")
    (tmp_path / ("x" * 65 + ".md")).write_text("Too long.", encoding="utf-8")
    (tmp_path / "..bad.md").write_text("Traversal.", encoding="utf-8")
    assert list_personas(tmp_path) == ["stack-chan"]
