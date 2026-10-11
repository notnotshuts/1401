"""Linux thermal zones and processors are valid evidence outside the _SB scope."""
import json
from pathlib import Path
import sys
import tempfile
import unittest

from p1401 import engine, linux_hardware, report


def hardware():
    return {
        'Motherboard': {'Name': 'Laptop fixture', 'Chipset': 'Alder Lake', 'Platform': 'Laptop'},
        'BIOS': {'Firmware Type': 'UEFI', 'Secure Boot': 'Disabled'},
        'CPU': {'Manufacturer': 'Intel', 'Processor Name': 'Intel Core i5-12450H',
                'Codename': 'Alder Lake-H', 'Core Count': '8', 'CPU Count': '1',
                'SIMD Features': 'SSE4.1, SSE4.2, AVX2'},
        'GPU': {'RTX 3050 Mobile': {'Manufacturer': 'NVIDIA', 'Codename': 'Ampere',
                                  'Device ID': '10DE-25A2', 'Device Type': 'Discrete GPU'},
                'Intel UHD': {'Manufacturer': 'Intel', 'Codename': 'Alder Lake-P',
                              'Device ID': '8086-46A3', 'Device Type': 'Integrated GPU'}},
        'Monitor': {'Panel': {'Connector Type': 'eDP', 'Resolution': '1920x1080', 'Connected GPU': 'Intel UHD'}},
        'Network': {}, 'USB Controllers': {}, 'Input': {},
        'Storage Controllers': {'NVMe': {'Bus Type': 'PCI', 'Device ID': '144D-A80B', 'Disk Drives': ['Fixture SSD']}},
        'System Devices': {'LNXTHERM': {'Bus Type': 'ACPI', 'ACPI Path': r'\_TZ_.TZ0_'}},
    }


class LinuxAcpiNamespace(unittest.TestCase):
    def validate(self, value):
        if engine.UPSTREAM not in sys.path: sys.path.insert(0, engine.UPSTREAM)
        from Scripts.report_validator import ReportValidator
        with tempfile.TemporaryDirectory() as td:
            path = Path(td) / 'Report.json'; path.write_text(json.dumps(value))
            return report.configure_validator(ReportValidator()).validate_report(str(path))

    def test_linux_system_scopes_survive_validation_unchanged(self):
        value = hardware()
        for path in (r'\_TZ_.TZ0_', r'\_TZ.TZ00', r'\_PR.CPU0', r'\_SB_.PCI0',
                     r'\_SB.PCI0.LPCB.EC0', r'\ABCD.DEV0', '\\', '_TZ.TZ0'):
            with self.subTest(path=path):
                value['System Devices']['LNXTHERM']['ACPI Path'] = path
                ok, errors, _, cleaned = self.validate(value)
                self.assertTrue(ok, errors)
                self.assertEqual(cleaned['System Devices']['LNXTHERM']['ACPI Path'], path)

    def test_malformed_system_paths_still_block_report(self):
        value = hardware()
        for path in ('', r'\_TZ_.', r'\_TZ_..TZ0_', r'\_TZ_.thermal', r'\_TZ_.TOOLONG',
                     r'\_TZ_.0BAD', r'\_TZ_.TZ0_\n', '\\_TZ_.TZ0_\n'):
            with self.subTest(path=path):
                value['System Devices']['LNXTHERM']['ACPI Path'] = path
                ok, errors, _, _ = self.validate(value)
                self.assertFalse(ok); self.assertTrue(any('ACPI Path' in e for e in errors))

    def test_bad_gpu_paths_are_not_relaxed_by_system_scope_fix(self):
        value = hardware(); value['GPU']['RTX 3050 Mobile']['ACPI Path'] = r'\_TZ_.TZ0_'
        ok, errors, _, _ = self.validate(value)
        self.assertFalse(ok); self.assertTrue(any('GPU' in e for e in errors))

    def test_thermal_zone_reaches_compatibility_and_panel_warning(self):
        with tempfile.TemporaryDirectory() as td:
            path = Path(td) / 'Report.json'; raw = json.dumps(hardware()); path.write_text(raw)
            result = linux_hardware.analyze(str(path))
            self.assertTrue(result['ok'], result['blockers'])
            self.assertEqual(result['native_versions'], [24])
            self.assertTrue(any('built-in screen on Intel UHD' in w for w in result['warnings']))
            self.assertEqual(path.read_text(), raw)


if __name__ == '__main__': unittest.main()
