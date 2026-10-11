"""Read-only hardware compatibility using the pinned OpCore-Simplify rules."""
import contextlib
import hashlib
import io
import json
from pathlib import Path
import re
import sys
import tempfile
from types import SimpleNamespace

from . import engine, nullmoth, report

VERSIONS = {19: 'Catalina 10.15', 20: 'Big Sur 11', 21: 'Monterey 12',
            22: 'Ventura 13', 23: 'Sonoma 14', 24: 'Sequoia 15', 25: 'Tahoe 26'}
SECTIONS = ('CPU', 'GPU', 'Sound', 'Biometric', 'Network', 'Storage Controllers', 'Bluetooth', 'SD Controller')
METHODS = ('cpu', 'gpu', 'sound', 'biometric', 'network', 'storage', 'bluetooth', 'sd_controller')
ANSI = re.compile(r'\x1b\[[0-9;]*[A-Za-z]')


def supports(compatibility, major):
    """Upstream device ranges are (maximum, minimum), including Darwin patch limits."""
    if not compatibility or len(compatibility) != 2 or not all(compatibility):
        return False
    def version(text):
        return tuple(int(x) for x in str(text).split('.'))
    return version(compatibility[1]) <= (int(major), 99, 99) and version(compatibility[0]) >= (int(major), 0, 0)


def devices(hardware, major):
    rows = []
    priority = ['CPU', 'GPU', 'Network', 'Storage Controllers', 'Sound', 'Bluetooth', 'Motherboard', 'BIOS']
    ordered = sorted(hardware, key=lambda key: priority.index(key) if key in priority else len(priority))
    for section in ordered:
        values = hardware[section]
        if not isinstance(values, dict): continue
        entries = [(values.get('Processor Name', 'CPU'), values)] if section == 'CPU' else list(values.items())
        if section in ('Motherboard', 'BIOS'):
            entries = [(values.get('Name', section), values)]
        for name, properties in entries:
            if not isinstance(properties, dict): continue
            compatibility = properties.get('Compatibility')
            patched = properties.get('OCLP Compatibility')
            if supports(compatibility, major):
                status = 'driver' if section == 'GPU' and nullmoth.supported(properties) else 'supported'
            elif supports(patched, major): status = 'patched'
            elif compatibility is not None: status = 'unsupported'
            else: status = 'unknown'
            rows.append({'section': section, 'name': name, 'status': status,
                         'native': compatibility, 'oclp': patched, 'details': properties})
    return rows


def analyze(report_path, major=24):
    """No ACPI compiler, downloads or EFI writes. Run in a child to isolate Scripts packages."""
    output = {'schema': 1, 'ok': False, 'target': int(major), 'report_path': str(Path(report_path).resolve()),
              'report_sha256': '', 'hardware': {}, 'devices': [], 'native_versions': [], 'oclp_versions': [],
              'blockers': [], 'warnings': [], 'transcript': '', 'source': 'Pinned OpCore-Simplify + 1401 NVIDIA policy'}
    transcript = io.StringIO()
    try:
        raw_bytes = Path(report_path).read_bytes()
        if len(raw_bytes) > 16 * 1024 * 1024: raise ValueError('Hardware report exceeds 16 MiB.')
        output['report_sha256'] = hashlib.sha256(raw_bytes).hexdigest()
        raw = json.loads(raw_bytes)
        if not isinstance(raw, dict): raise ValueError('Hardware report must be a JSON object.')
        if engine.UPSTREAM not in sys.path: sys.path.insert(0, engine.UPSTREAM)
        from Scripts.compatibility_checker import CompatibilityChecker
        from Scripts.report_validator import ReportValidator
        from Scripts.datasets import os_data
        with tempfile.TemporaryDirectory(prefix='1401-check-') as td:
            normalized, notes = report.normalized_copy(report_path, td, raw_hardware=raw)
            valid, errors, warnings, hardware = ReportValidator().validate_report(normalized)
        output['warnings'] = notes + warnings
        output['hardware'] = hardware or raw
        if not valid or errors:
            output['blockers'].extend(errors)
            output['devices'] = devices(output['hardware'], major)
            return output
        checker = CompatibilityChecker()
        checker.hardware_report = hardware
        checker.min_native_macos_version = os_data.get_lowest_darwin_version()
        checker.max_native_macos_version = os_data.get_latest_darwin_version()
        checker.ocl_patched_macos_version = None
        checker.utils.head = lambda *a, **k: None
        checker.utils.request_input = lambda *a, **k: ''
        def stop(*a, **k): raise engine.EngineExit('Compatibility gate stopped')
        checker.utils.exit_program = stop
        notices = SimpleNamespace(notices=[])
        engine._nullmoth_gpu_pass(checker, notices)
        with contextlib.redirect_stdout(transcript):
            for section, method in zip(SECTIONS, METHODS):
                if not hardware.get(section):
                    if section in ('CPU', 'GPU', 'Storage Controllers'):
                        output['blockers'].append(f'Missing required hardware: {section}')
                    continue
                before = transcript.tell()
                try:
                    getattr(checker, 'check_' + method + '_compatibility')()
                except engine.EngineExit:
                    detail = ANSI.sub('', transcript.getvalue()[before:]).strip()
                    output['blockers'].append(f'{section}: {detail or "unsupported hardware"}')
        output['warnings'].extend(notices.notices)
        low, high = checker.min_native_macos_version, checker.max_native_macos_version
        if low and high and not output['blockers']:
            output['native_versions'] = [n for n in VERSIONS if supports((high, low), n)]
        # OCLP is an alternative only within the CPU range. It never clears a fatal hardware gate.
        if checker.ocl_patched_macos_version and not output['blockers']:
            cpu_range = hardware['CPU'].get('Compatibility')
            output['oclp_versions'] = [n for n in VERSIONS if supports(checker.ocl_patched_macos_version, n) and supports(cpu_range, n)]
        output['hardware'] = hardware
        output['devices'] = devices(hardware, major)
        output['ok'] = not output['blockers'] and major in output['native_versions'] + output['oclp_versions']
        if not output['ok'] and not output['blockers']:
            output['blockers'].append(f'The selected macOS ({VERSIONS.get(major, major)}) is outside the supported hardware range.')
    except Exception as error:
        output['blockers'].append(f'{type(error).__name__}: {error}')
    finally:
        output['transcript'] = ANSI.sub('', transcript.getvalue())[-30000:]
    return output


def save(output, destination):
    path = Path(destination)
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(path.suffix + '.part')
    temporary.write_text(json.dumps(output, indent=2, ensure_ascii=False), encoding='utf-8')
    temporary.replace(path)
