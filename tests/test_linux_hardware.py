import json
from pathlib import Path
import tempfile
import unittest
from p1401 import linux_hardware as hw


class HardwareCompatibility(unittest.TestCase):
    def test_range_order_and_unknown_are_not_supported(self):
        self.assertTrue(hw.supports(('24.99.99', '20.0.0'), 24))
        self.assertFalse(hw.supports(('24.99.99', '20.0.0'), 25))
        self.assertFalse(hw.supports((None, None), 24))
        self.assertFalse(hw.supports(None, 24))

    def test_status_distinguishes_oclp_unknown_and_unsupported(self):
        rows = hw.devices({'GPU': {'Old GPU': {'Compatibility': [19, 17], 'OCLP Compatibility': ['24.99.99', '20.0.0']}, 'No driver': {'Compatibility': [None, None]}}, 'Network': {'Unknown': {'Device ID': 'AAAA-BBBB'}}}, 24)
        self.assertEqual([r['status'] for r in rows], ['patched', 'unsupported', 'unknown'])

    def test_invalid_report_returns_blockers_without_downloads(self):
        with tempfile.TemporaryDirectory() as td:
            path = Path(td) / 'Report.json'; path.write_text(json.dumps({'CPU': {}}))
            result = hw.analyze(str(path))
            self.assertFalse(result['ok']); self.assertTrue(result['blockers'])
            self.assertEqual(path.read_text(), json.dumps({'CPU': {}}))

    def test_malformed_report_is_structured_failure(self):
        with tempfile.TemporaryDirectory() as td:
            path = Path(td) / 'Report.json'; path.write_text('broken')
            self.assertFalse(hw.analyze(str(path))['ok'])


if __name__ == '__main__': unittest.main()

class VersionIntersection(unittest.TestCase):
    def hardware(self, gpus):
        return {'CPU': {'Compatibility': [25, 19]}, 'GPU': gpus,
                'Storage Controllers': {'SSD': {'Bus Type': 'PCI', 'Disk Drives': ['SSD'], 'Compatibility': [25, 19]}}}

    def test_gpu_gaps_not_bridged_by_aggregate_range(self):
        h = self.hardware({'old': {'Compatibility': [20, 19]}, 'new': {'Compatibility': [25, 24]}})
        native, patched = hw.compatible_versions(h, [25, 19], None)
        self.assertEqual(native, [19, 20, 24, 25]); self.assertEqual(patched, [])

    def test_wifi_patch_cannot_rescue_unsupported_gpu(self):
        h = self.hardware({'old': {'Compatibility': [22, 19]}})
        self.assertEqual(hw.compatible_versions(h, [22, 19], [25, 23]), ([19, 20, 21, 22], []))

    def test_gpu_patch_obeys_storage_and_cpu_limits(self):
        h = self.hardware({'old': {'Compatibility': [22, 19], 'OCLP Compatibility': [25, 23]}})
        h['CPU']['Compatibility'] = [24, 19]
        h['Storage Controllers']['SSD']['Compatibility'] = [23, 19]
        self.assertEqual(hw.compatible_versions(h, [22, 19], [25, 23]), ([19, 20, 21, 22], [23]))

    def test_unobserved_storage_is_not_success(self):
        h = self.hardware({'new': {'Compatibility': [25, 19]}})
        del h['Storage Controllers']['SSD']['Disk Drives']
        self.assertEqual(hw.compatible_versions(h, [25, 19], None), ([], []))

    def test_malformed_and_reversed_ranges(self):
        for value in ['bad', ['oops', 19], [19, 25], {}, [25, None], [[], 19]]:
            with self.subTest(value=value): self.assertFalse(hw.supports(value, 24))
        self.assertTrue(hw.supports([24, 24], 24))
