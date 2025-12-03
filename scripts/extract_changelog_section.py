#!/usr/bin/env python3
"""Extract a version section from CHANGELOG.md for GitHub Releases."""

from __future__ import annotations

import re
import sys
from pathlib import Path


def parse_args() -> tuple[str, Path, Path]:
    if len(sys.argv) != 3:
        print("usage: scripts/extract_changelog_section.py <version> <output_path>", file=sys.stderr)
        sys.exit(1)

    version = sys.argv[1].lstrip("v")
    if not version:
        print("error: version must be non-empty", file=sys.stderr)
        sys.exit(1)

    output_path = Path(sys.argv[2])
    changelog_path = Path(__file__).resolve().parent.parent / "CHANGELOG.md"
    if not changelog_path.is_file():
        print(f"error: changelog not found at {changelog_path}", file=sys.stderr)
        sys.exit(1)

    return version, changelog_path, output_path


def extract_section(contents: str, version: str) -> tuple[str, str]:
    heading_pattern = re.compile(
        rf"^##\s+\[?{re.escape(version)}\]?(?:\s*-\s*\d{{4}}-\d{{2}}-\d{{2}})?\s*$",
        re.MULTILINE,
    )
    match = heading_pattern.search(contents)
    if not match:
        raise ValueError(f"changelog entry for {version} not found")

    heading = match.group(0).strip()
    section_start = match.end()
    next_heading = re.search(r"^##\s+", contents[section_start:], re.MULTILINE)
    section_end = section_start + next_heading.start() if next_heading else len(contents)
    body = contents[section_start:section_end].strip()

    if not body:
        raise ValueError(f"changelog entry for {version} is empty")

    return heading, body


def main() -> None:
    version, changelog_path, output_path = parse_args()
    contents = changelog_path.read_text(encoding="utf-8")

    try:
        heading, body = extract_section(contents, version)
    except ValueError as err:
        print(f"error: {err}", file=sys.stderr)
        sys.exit(1)

    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_text(f"{heading}\n\n{body}\n", encoding="utf-8")


if __name__ == "__main__":
    main()
