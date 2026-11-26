#!/usr/bin/env python3
"""Print one version's section of CHANGELOG.md, for the release notes.

    scripts/changelog_section.py 0.4.0        # the text under "## 0.4.0 ..."
    scripts/changelog_section.py --self-test

A section starts at a line "## <version>" (anything may follow the version, such as
" - 2026-12-01") and ends before the next "## " line. Exits with status 1 when the
version has no section or the section is empty, so a release cannot go out without
its notes.
"""
import pathlib
import re
import sys


def section(text: str, version: str) -> str | None:
    lines = text.splitlines()
    head = re.compile(r"^## v?" + re.escape(version) + r"(\s|$)")
    out: list[str] = []
    inside = False
    for line in lines:
        if line.startswith("## "):
            if inside:
                break
            inside = bool(head.match(line))
            continue
        if inside:
            out.append(line)
    body = "\n".join(out).strip()
    return body or None


def self_test() -> None:
    text = """# Changelog

## Unreleased

* Next

## 0.4.0 - 2026-12-01

### New

* Key-ins

## 0.3.0 - 2026-11-26

* Locust
"""
    assert section(text, "0.4.0") == "### New\n\n* Key-ins", section(text, "0.4.0")
    assert section(text, "0.3.0") == "* Locust"
    assert section(text, "Unreleased") == "* Next"
    assert section(text, "0.4") is None, "a prefix of a version is not that version"
    assert section(text, "9.9.9") is None
    print("ok")


def main() -> int:
    if len(sys.argv) != 2:
        print(__doc__.strip(), file=sys.stderr)
        return 2
    if sys.argv[1] == "--self-test":
        self_test()
        return 0
    version = sys.argv[1].removeprefix("v")
    path = pathlib.Path(__file__).resolve().parent.parent / "CHANGELOG.md"
    body = section(path.read_text(encoding="utf-8"), version)
    if body is None:
        print(f"CHANGELOG.md has no section for {version}: add '## {version} - <date>' with the release notes.", file=sys.stderr)
        return 1
    print(body)
    return 0


if __name__ == "__main__":
    sys.exit(main())
