#!/usr/bin/env python3
"""Read or replace the [workspace.package] version in a Cargo.toml.

The product version lives only in that table. Other version keys are left alone.
"""

from __future__ import annotations

import argparse
import pathlib
import re
import sys

SECTION = "[workspace.package]"
VERSION_LINE = re.compile(r'^version\s*=\s*"[^"]*"\s*$')
SEMVER = re.compile(r"^\d+\.\d+\.\d+$")


def read_version(text: str) -> str:
    in_section = False
    for line in text.splitlines():
        stripped = line.strip()
        if stripped.startswith("[") and stripped.endswith("]"):
            in_section = stripped == SECTION
            continue
        if in_section and VERSION_LINE.match(stripped):
            return stripped.split('"', 2)[1]
    raise SystemExit(f"no version key in {SECTION}")


def write_version(text: str, version: str) -> str:
    lines = text.splitlines(keepends=True)
    in_section = False
    replaced = False
    output: list[str] = []
    for line in lines:
        stripped = line.strip()
        if stripped.startswith("[") and stripped.endswith("]"):
            in_section = stripped == SECTION
        if in_section and not replaced and VERSION_LINE.match(stripped):
            newline = "\n" if line.endswith("\n") else ""
            output.append(f'version = "{version}"{newline}')
            replaced = True
            continue
        output.append(line)
    if not replaced:
        raise SystemExit(f"no version key in {SECTION}")
    return "".join(output)


def normalize(version: str) -> str:
    value = version.strip()
    if value.startswith("v"):
        value = value[1:]
    if not SEMVER.fullmatch(value):
        raise SystemExit(f"version must be semver X.Y.Z, got {version!r}")
    return value


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "action",
        choices=("read", "write"),
        help="read the current version, or write a new one",
    )
    parser.add_argument("version", nargs="?", help="semver to write, with an optional v prefix")
    parser.add_argument(
        "--manifest",
        type=pathlib.Path,
        default=pathlib.Path("Cargo.toml"),
        help="path to the workspace Cargo.toml",
    )
    args = parser.parse_intermixed_args(argv)
    text = args.manifest.read_text()
    if args.action == "read":
        print(read_version(text))
        return 0
    if args.version is None:
        raise SystemExit("write requires a version")
    version = normalize(args.version)
    args.manifest.write_text(write_version(text, version))
    print(version)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
