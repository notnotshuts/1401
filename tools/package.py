#!/usr/bin/env python3
"""Package built Ferrite binaries with their CLI, guides and license notices."""
from __future__ import annotations

import argparse
from pathlib import Path
import shutil
import tarfile
import tempfile
import tomllib
import zipfile

ROOT = Path(__file__).resolve().parents[1]


def package(platform: str) -> Path:
    version = tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]["package"]["version"]
    label = f"Ferrite-{version}-{platform}-x86_64"
    dest = ROOT / "dist"
    dest.mkdir(exist_ok=True)
    suffix = ".exe" if platform == "windows" else ""
    with tempfile.TemporaryDirectory(prefix="ferrite-package-") as temporary:
        stage = Path(temporary) / label
        stage.mkdir()
        for name in ["ferrite", "ferrite-cli"]:
            source = ROOT / "target/release" / f"{name}{suffix}"
            if not source.is_file():
                raise FileNotFoundError(f"Build first: missing {source}")
            shutil.copy2(source, stage / source.name)
            if platform == "linux":
                (stage / source.name).chmod(0o755)
        for name in ["LICENSE", "NOTICE.md", "README.md"]:
            shutil.copy2(ROOT / name, stage / name)
        shutil.copytree(ROOT / "licenses", stage / "licenses")
        guides = stage / "docs/ferrite"
        guides.mkdir(parents=True)
        for language in ["ru", "en", "es"]:
            shutil.copy2(ROOT / "docs/ferrite" / f"Guide-{language}.md", guides)
        # README links remain usable in offline packages.
        for name in ["MIGRATION.md", "VALIDATION.md", "zed-overview-ru.png", "opencore-qemu.png"]:
            shutil.copy2(ROOT / "docs/ferrite" / name, guides)
        if platform == "windows":
            archive = dest / f"{label}.zip"
            with zipfile.ZipFile(archive, "w", compression=zipfile.ZIP_DEFLATED) as output:
                for path in sorted(stage.rglob("*")):
                    if path.is_file():
                        output.write(path, path.relative_to(stage.parent))
        else:
            archive = dest / f"{label}.tar.gz"
            with tarfile.open(archive, "w:gz") as output:
                output.add(stage, arcname=label)
    return archive


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--platform", choices=["linux", "windows"], required=True)
    print(package(parser.parse_args().platform))
