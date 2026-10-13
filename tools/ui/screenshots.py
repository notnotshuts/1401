#!/usr/bin/env python3
"""Render screenshot-only UI states; synthetic data, no hardware operations."""
import argparse
import os
from pathlib import Path
import subprocess

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("binary", type=Path)
parser.add_argument("output", type=Path)
args = parser.parse_args()
args.output.mkdir(parents=True, exist_ok=True)
cases = [
    ("overview-light", "overview", "light", "ru", False, False),
    ("overview-dark", "overview", "dark", "ru", False, False),
    ("hardware-light", "hardware", "light", "ru", False, True),
    ("usb-light", "usb", "light", "ru", False, True),
    ("usb-compact", "usb", "light", "ru", True, True),
    ("guide-dark-es", "guide", "dark", "es", False, True),
    ("settings-dark", "settings", "dark", "ru", False, True),
    ("efi-light-en", "efi", "light", "en", False, True),
]
for name, page, theme, language, compact, demo in cases:
    image = (args.output / f"{name}.png").resolve()
    env = os.environ.copy()
    env.update(FERRITE_SCREENSHOT_TO=str(image), FERRITE_PREVIEW_PAGE=page,
               FERRITE_PREVIEW_THEME=theme, FERRITE_PREVIEW_LANGUAGE=language,
               FERRITE_PREVIEW_COMPACT="1" if compact else "0",
               FERRITE_PREVIEW_DEMO="1" if demo else "0")
    subprocess.run([str(args.binary.resolve())], env=env, check=True, timeout=30)
    data = image.read_bytes()
    if not data.startswith(b"\x89PNG\r\n\x1a\n") or len(data) < 1000:
        raise RuntimeError(f"Invalid rendered screenshot: {image}")
    print(image, flush=True)
