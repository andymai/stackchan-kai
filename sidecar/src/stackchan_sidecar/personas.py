from pathlib import Path


def load_persona(name: str, personas_dir: Path) -> str:
    path = personas_dir / f"{name}.md"
    if not path.is_file():
        raise FileNotFoundError(f"persona file not found: {path}")
    text = path.read_text(encoding="utf-8")
    return _strip_frontmatter(text).strip()


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
