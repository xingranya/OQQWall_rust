#!/usr/bin/env python3
import argparse
import os
from pathlib import Path
import zipfile


EXCLUDE_DIRS = {".git", "target", "res"}
EXCLUDE_EXTS = {".png", ".svg", ".res"}


def is_text_file(path: Path) -> bool:
    try:
        with path.open("rb") as f:
            chunk = f.read(8192)
    except OSError:
        return False
    if b"\x00" in chunk:
        return False
    if not chunk:
        return True
    try:
        chunk.decode("utf-8")
        return True
    except UnicodeDecodeError:
        return False


def collect_files(repo_root: Path) -> list[Path]:
    files: list[Path] = []
    for root, dirs, filenames in os.walk(repo_root):
        dirs[:] = [d for d in dirs if d not in EXCLUDE_DIRS]
        for name in filenames:
            path = Path(root) / name
            if path.suffix.lower() in EXCLUDE_EXTS:
                continue
            if is_text_file(path):
                files.append(path)
    return sorted(files)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("repo_root", help="Workspace root")
    parser.add_argument("output_zip", help="Zip output path")
    args = parser.parse_args()

    repo_root = Path(args.repo_root).resolve()
    output_zip = Path(args.output_zip).resolve()
    output_zip.parent.mkdir(parents=True, exist_ok=True)

    files = collect_files(repo_root)
    with zipfile.ZipFile(output_zip, "w", compression=zipfile.ZIP_DEFLATED) as zf:
        for path in files:
            zf.write(path, path.relative_to(repo_root))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
