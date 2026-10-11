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
