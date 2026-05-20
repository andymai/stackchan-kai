from pathlib import Path

# Max length of a persona slug. Mirrors the firmware-side
# PERSONA_NAME_MAX_BYTES in stackchan-net::config so the wire boundary
# rejects the same set of inputs on both ends.
_PERSONA_NAME_MAX_BYTES = 64


def load_persona(name: str, personas_dir: Path) -> str:
    """Load the persona prompt at ``{personas_dir}/{name}.md``.

    Raises ``ValueError`` if ``name`` isn't a safe filename component
    (controls path traversal + HTTP header injection on the
    firmware-supplied input path), or ``FileNotFoundError`` if the
    resolved file doesn't exist.
    """
    _validate_slug(name)
    path = personas_dir / f"{name}.md"
    if not path.is_file():
        raise FileNotFoundError(f"persona file not found: {path}")
    text = path.read_text(encoding="utf-8")
    return _strip_frontmatter(text).strip()


def _validate_slug(name: str) -> None:
    """Reject persona slugs that could traverse the filesystem or
    inject HTTP headers (relevant when ``name`` came in from an
    ``X-Persona-Name`` request header).

    Defence in depth — the firmware-side validator in
    ``stackchan-net::config::validate_persona_name`` applies the
    same rules at config time, but the sidecar can't trust an
    upstream proxy or a curl-based caller to have gone through
    that gate.
    """
    if not name:
        raise ValueError("persona name must be non-empty")
    if len(name.encode("utf-8")) > _PERSONA_NAME_MAX_BYTES:
        raise ValueError(
            f"persona name exceeds {_PERSONA_NAME_MAX_BYTES} bytes"
        )
    if any(ord(c) < 0x20 or ord(c) == 0x7F for c in name):
        raise ValueError("persona name contains a control character")
    if "/" in name or "\\" in name or ".." in name:
        raise ValueError(
            "persona name contains a path separator or traversal token"
        )


def _strip_frontmatter(text: str) -> str:
    if not text.startswith("---"):
        return text
    lines = text.splitlines()
    if len(lines) < 2 or lines[0].strip() != "---":
        return text
    for idx in range(1, len(lines)):
        if lines[idx].strip() == "---":
            return "\n".join(lines[idx + 1 :])
    return text
