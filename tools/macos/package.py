#!/usr/bin/env python3
"""Development-only macOS .app/DMG packaging. Python is not used by the app."""
from __future__ import annotations
import argparse
import os
import pathlib
import platform
import plistlib
import re
import shutil
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]

def package(target: str, skip_build: bool) -> pathlib.Path:
    if sys.platform != "darwin":
        raise RuntimeError("Run this packager on macOS with Xcode Command Line Tools and Rust installed")
    native = "aarch64-apple-darwin" if platform.machine() == "arm64" else "x86_64-apple-darwin"
    targets = ["aarch64-apple-darwin", "x86_64-apple-darwin"] if target == "universal" else [native if target == "native" else target]
    version = re.search(r'^version = "([0-9.]+)"$', (ROOT / "Cargo.toml").read_text(), re.MULTILINE).group(1)
    label = "universal" if len(targets) == 2 else targets[0].split("-")[0]
    dest = ROOT / "dist/macos" / label
    dest.mkdir(parents=True, exist_ok=True)
    bundle = dest / "Ferrite.app"
    if bundle.exists():
        shutil.rmtree(bundle)
    binaries = bundle / "Contents/MacOS"
    resources = bundle / "Contents/Resources"
    binaries.mkdir(parents=True)
    resources.mkdir(parents=True)
    env = os.environ.copy()
    env.setdefault("MACOSX_DEPLOYMENT_TARGET", "12.0")
    for arch in targets:
        if not skip_build:
            subprocess.run(["rustup", "target", "add", arch], check=True)
            subprocess.run(["cargo", "build", "--release", "--locked", "--workspace", "--target", arch], cwd=ROOT, env=env, check=True)
    for name in ["ferrite", "ferrite-cli"]:
        inputs = [ROOT / "target" / arch / "release" / name for arch in targets]
        if len(inputs) == 2:
            subprocess.run(["lipo", "-create", *map(str, inputs), "-output", str(binaries / name)], check=True)
        else:
            shutil.copy2(inputs[0], binaries / name)
        (binaries / name).chmod(0o755)
    info = {"CFBundleName": "Ferrite", "CFBundleDisplayName": "Ferrite", "CFBundleIdentifier": "app.ferrite.desktop", "CFBundleExecutable": "ferrite", "CFBundlePackageType": "APPL", "CFBundleShortVersionString": version, "CFBundleVersion": version, "LSMinimumSystemVersion": "12.0", "NSHighResolutionCapable": True, "CFBundleDevelopmentRegion": "en", "CFBundleLocalizations": ["ru", "en", "es"]}
    with (bundle / "Contents/Info.plist").open("wb") as file:
        plistlib.dump(info, file)
    shutil.copy2(ROOT / "LICENSE", resources / "LICENSE")
    shutil.copy2(ROOT / "NOTICE.md", resources / "NOTICE.md")
    shutil.copytree(ROOT / "licenses", resources / "licenses")
    for language in ["ru", "en", "es"]:
        shutil.copy2(ROOT / "docs/ferrite" / f"Guide-{language}.md", resources / f"Guide-{language}.md")
    subprocess.run(["plutil", "-lint", str(bundle / "Contents/Info.plist")], check=True)
    # Local ad-hoc signing permits the Apple Silicon bundle to execute; this is not notarization.
    subprocess.run(["codesign", "--force", "--deep", "--sign", "-", str(bundle)], check=True)
    subprocess.run(["codesign", "--verify", "--deep", "--strict", str(bundle)], check=True)
    dmg = dest / f"Ferrite-{version}-{label}.dmg"
    image_root = dest / "image"
    if image_root.exists():
        shutil.rmtree(image_root)
    image_root.mkdir()
    shutil.copytree(bundle, image_root / "Ferrite.app", copy_function=shutil.copy2)
    (image_root / "Applications").symlink_to("/Applications")
    if dmg.exists():
        dmg.unlink()
    subprocess.run(["hdiutil", "create", "-volname", "Ferrite", "-srcfolder", str(image_root), "-format", "UDZO", str(dmg)], check=True)
    return dmg

if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", choices=["native", "universal", "aarch64-apple-darwin", "x86_64-apple-darwin"], default="native")
    parser.add_argument("--skip-build", action="store_true")
    args = parser.parse_args()
    dmg = package(args.target, args.skip_build)
    print(dmg)
    if output := os.environ.get("GITHUB_OUTPUT"):
        with open(output, "a", encoding="utf-8") as file:
            file.write(f"dmg={dmg}\n")
