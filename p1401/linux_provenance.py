"""Carry OpenCore archive provenance across ordinary-user build / root USB write."""
import contextlib
import json
from pathlib import Path
import re
from urllib.parse import urlsplit

from . import machine_handoff, validate


def record(build, report):
    profile = machine_handoff.verified(build, report)
    url, sha = validate._opencore_release()
    checked_release({'url': url, 'sha256': sha})
    profile['linux_opencore_release'] = {'url': url, 'sha256': sha}
    destination = Path(build) / machine_handoff.NAME
    temporary = destination.with_suffix('.part')
    temporary.write_text(json.dumps(profile, indent=1, sort_keys=True), encoding='utf-8')
    temporary.replace(destination)


def checked_release(value):
    url = value.get('url', ''); sha = value.get('sha256', '')
    parsed = urlsplit(url)
    allowed = ('/acidanthera/OpenCorePkg/releases/download/', '/dortania/build-repo/releases/download/')
    if (parsed.scheme != 'https' or parsed.netloc != 'github.com' or
            not parsed.path.startswith(allowed) or not parsed.path.endswith('.zip') or
            parsed.query or parsed.fragment or not re.fullmatch(r'[0-9a-f]{64}', sha)):
        raise RuntimeError('Invalid Linux OpenCore release provenance. Rebuild the EFI.')
    return url, sha


@contextlib.contextmanager
def for_profile(profile):
    release = checked_release(profile.get('linux_opencore_release', {}))
    # No user-owned Python code or cached executable is used as root. The
    # validator extracts fresh bytes from the recorded official release archive.
    original = validate._opencore_release
    validate._opencore_release = lambda: release
    try:
        yield
    finally:
        validate._opencore_release = original
