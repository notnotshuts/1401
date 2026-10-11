import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch
from p1401 import linux_scan, report, scan


class LinuxScanRegression(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(); self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)

    def put(self, name, value):
        path = self.root / name; path.parent.mkdir(parents=True, exist_ok=True); path.write_text(value)
        return path

    def test_cpu_physical_cores_not_logical_threads_and_sse3_alias(self):
        self.put('proc/cpuinfo', '\n\n'.join(f'processor : {i}\nvendor_id : AuthenticAMD\nmodel name : Ryzen\ncpu family : 25\nmodel : 33\nstepping : 0\nphysical id : 0\ncore id : {i//2}\nflags : sse sse2 pni ssse3 sse4_1 sse4_2 avx avx2' for i in range(16)))
        value = linux_scan.cpu(SimpleNamespace(lookup_codename=lambda *a: 'Vermeer'), self.root)
        self.assertEqual(value['Core Count'], '8'); self.assertEqual(value['Thread Count'], '16')
        self.assertEqual(value['CPU Count'], '1'); self.assertIn('SSE3, SSSE3', value['SIMD Features'])

    def test_cpu_falls_back_to_kernel_topology(self):
        self.put('proc/cpuinfo', 'processor : 0\nvendor_id : GenuineIntel\nmodel name : Intel\nflags : sse4_2')
        self.put('sys/devices/system/cpu/cpu0/topology/core_id', '7')
        self.put('sys/devices/system/cpu/cpu0/topology/physical_package_id', '0')
        self.assertEqual(linux_scan.cpu(SimpleNamespace(lookup_codename=lambda *a: 'Unknown'), self.root)['Core Count'], '1')

    def test_missing_topology_never_assumes_threads_are_cores(self):
        self.put('proc/cpuinfo', 'processor : 0\nvendor_id : GenuineIntel')
        with self.assertRaisesRegex(ValueError, 'физические ядра'): linux_scan.cpu(SimpleNamespace(), self.root)

    def info(self):
        info = SimpleNamespace(utils=SimpleNamespace(head=lambda *a: None), devices_by_class={},
                               gpu=lambda: {}, storage_controllers=lambda: {}, bios=lambda: {})
        for _, method, _ in linux_scan.STEPS:
            if not hasattr(info, method): setattr(info, method, lambda: {})
        return info

    def test_optional_section_exception_preserves_report_and_diagnostics(self):
        info = self.info(); info.network = lambda: (_ for _ in ()).throw(OSError('test read failure'))
        checkpoints = []
        linux_scan.install(info, lambda: checkpoints.append(dict(info.result)))
        with patch.object(linux_scan, 'cpu', return_value={'Processor Name': 'Fixture'}), patch.object(linux_scan, 'motherboard', return_value={'Name': 'Fixture'}):
            info.hardware_collector()
        self.assertEqual(info.result['CPU']['Processor Name'], 'Fixture')
        self.assertEqual(info.result['Network'], {})
        self.assertTrue(any('Network' in s for s in info.result['Scan Errors']))
        self.assertEqual(len(checkpoints), len(linux_scan.STEPS))
        self.assertIn('Storage Controllers', info.result)

    def test_missing_required_optional_groups_normalized_but_bad_data_not_dropped(self):
        value, _ = report.normalize({'CPU': {}, 'GPU': {}, 'Network': {'bad': {'Device ID': 'BAD'}}})
        self.assertEqual(value['Network']['bad']['Device ID'], 'BAD')
        self.assertEqual(value['USB Controllers'], {}); self.assertEqual(value['Input'], {})
        self.assertNotIn('Storage Controllers', value)

    def test_bom_import_normalized_in_private_copy(self):
        source = self.put('Report.json', '\ufeff' + json.dumps({'CPU': {}, 'GPU': {}}))
        (self.root / 'normalized').mkdir()
        destination, _ = report.normalized_copy(str(source), str(self.root / 'normalized'))
        self.assertEqual(json.loads(Path(destination).read_text())['Network'], {})
        self.assertTrue(source.read_text().startswith('\ufeff'))

    def test_scan_json_array_is_diagnostic_not_attribute_error(self):
        self.put('Report.json', '[]')
        failures = scan.check(self.root)
        self.assertTrue(any('hardware object' in f for f in failures))

    def test_secure_boot_binary_flag_and_unreadable_is_unknown(self):
        info = self.info(); linux_scan.install(info); info.notes = []
        with patch.object(Path, 'read_bytes', return_value=b'\x06\x00\x00\x00\x01'):
            self.assertEqual(info.bios()['Secure Boot'], 'Enabled')
        with patch.object(Path, 'read_bytes', side_effect=OSError('no efivars')):
            self.assertEqual(info.bios()['Secure Boot'], 'Unknown')
        self.assertTrue(info.notes)

    def test_pci_gpu_without_drm_driver_is_reported_and_not_duplicated(self):
        path = self.root / '0000:01:00.0'; path.mkdir()
        for name, value in [('class', '0x030000'), ('vendor', '0x10de'), ('device', '0x2204')]: (path / name).write_text(value)
        info = SimpleNamespace(get_device_location_paths=lambda _: {'PCI Path': 'PciRoot(0x0)/Pci(0x1,0x0)'},
                               classify_gpu=lambda _: {'Manufacturer': 'NVIDIA', 'Codename': 'Ampere', 'Device Type': 'Discrete GPU'},
                               get_pci_device_name_and_class=lambda _: ('RTX 3090', 'VGA compatible controller'),
                               utils=SimpleNamespace(get_unique_key=lambda name, d: name))
        found = linux_scan.gpu_fallback(info, {}, self.root)
        self.assertEqual(found['RTX 3090']['Device ID'], '10DE-2204')
        self.assertEqual(len(linux_scan.gpu_fallback(info, found, self.root)), 1)

    def test_invalid_monitor_edid_does_not_abort_gpu_scan(self):
        edid = self.put('drm/card0-HDMI-A-1/edid', '')
        self.put('drm/card0-HDMI-A-1/status', 'connected')
        self.put('drm/card0-HDMI-A-1/modes', '1920x1080\n1280x720')
        device = str(self.root / 'drm')
        info = SimpleNamespace(result={'GPU': {'Card': {'Device Path': device}}},
                               utils=SimpleNamespace(find_matching_paths=lambda *a, **k: [('card0-HDMI-A-1/edid', 'file')],
                                                     get_unique_key=lambda name, d: name), parse_edid=lambda _: {})
        monitor = next(iter(linux_scan.monitor(info).values()))
        self.assertEqual(monitor['Connector Type'], 'HDMI'); self.assertEqual(monitor['Resolution'], '1920x1080')
        self.assertEqual(monitor['Connected GPU'], 'Card')
