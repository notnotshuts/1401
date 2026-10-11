import unittest
from unittest.mock import patch
from p1401 import guide, linux_guide


class LinuxGuide(unittest.TestCase):
    def test_personal_guide_uses_linux_actions_and_exact_settings(self):
        settings = [guide.Setting('CFG Lock (MSR 0xE2 write protection)', 'Off', False, 'workaround'),
                    guide.Setting('Resizable BAR / Re-Size BAR', 'Leave it Disabled', True, 'exact setting')]
        with patch.object(guide, 'bios_settings', return_value=settings):
            text = linux_guide.render({'Motherboard': {'Name': 'MSI Board'}}, {}, {'macos_version': '24.6.0'})
        self.assertIn('Sequoia', text); self.assertIn('MSI Board', text)
        self.assertIn('оставить выключенным', text); self.assertIn('обход CFG Lock', text)
        self.assertIn('не приостанавливает BitLocker', text)
        self.assertNotIn('Restart into BIOS setup', text)
