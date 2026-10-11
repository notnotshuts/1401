import copy
import unittest
from unittest.mock import patch, Mock
from p1401 import linux_usb as usb


def fixture(**extra):
    return {'path': '/dev/sdz', 'type': 'disk', 'size': 16 * 10**9, 'model': 'Test USB',
            'serial': 'serial-1', 'tran': 'usb', 'ro': False, 'maj:min': '8:240',
            'mountpoints': [], 'children': [], **extra}


class LinuxDiskSafety(unittest.TestCase):
    def test_internal_and_os_disks_are_refused(self):
        cases = [fixture(tran='nvme'), fixture(ro=True), fixture(size=1024),
                 fixture(mountpoints=['/']), fixture(mountpoints=['[SWAP]']),
                 fixture(children=[fixture(type='part', fstype='ext4')]),
                 fixture(children=[fixture(type='part', fstype='ntfs')]),
                 fixture(children=[fixture(type='part', parttype='c12a7328-f81f-11d2-ba4b-00a0c93ec93b')]),
                 fixture(children=[fixture(type='crypt')])]
        for node in cases:
            with self.subTest(node=node): self.assertFalse(usb.eligible(usb.describe(node))[0])

    def test_ordinary_mounted_fat_usb_can_be_selected(self):
        node = fixture(children=[fixture(type='part', fstype='vfat', mountpoints=['/media/user/USB'])])
        self.assertTrue(usb.eligible(usb.describe(node))[0])

    def test_replacement_and_missing_identity_stop_before_open(self):
        selected = usb.describe(fixture())
        for key, value in [('SerialNumber', 'replacement'), ('SerialNumber', ''), ('Size', 123), ('DeviceId', '8:241')]:
            changed = {**selected, key: value}
            with patch.object(usb, 'list_disks', return_value=[changed]), patch.object(usb.os, 'stat') as st:
                with self.assertRaises(usb.UsbError): usb.check_disk(selected)
                st.assert_not_called()

    def test_active_holders_are_refused(self):
        disk = usb.describe(fixture())
        with patch.object(usb, 'list_disks', return_value=[disk]), patch.object(usb.os, 'stat', return_value=Mock(st_mode=0o60600, st_rdev=0)), patch.object(usb.os, 'major', return_value=8), patch.object(usb.os, 'minor', return_value=240), patch.object(usb.Path, 'is_dir', return_value=True), patch.object(usb.Path, 'iterdir', return_value=iter(['dm-0'])):
            with self.assertRaisesRegex(usb.UsbError, 'in use'): usb.check_disk(disk)

    def test_invalid_build_never_touches_disk(self):
        with patch.object(usb.os, 'geteuid', return_value=0), patch.object(usb.shutil, 'which', return_value='/bin/tool'), patch('p1401.validate.validate', return_value={'ok': False}), patch.object(usb, 'check_disk') as check, patch.object(usb, 'format_disk') as fmt:
            with self.assertRaisesRegex(usb.UsbError, 'validation'): usb.write({}, '/missing', {})
            check.assert_not_called(); fmt.assert_not_called()

    def test_failed_download_never_touches_disk(self):
        profile = {'fixture': True}
        with patch.object(usb.os, 'geteuid', return_value=0), patch.object(usb.shutil, 'which', return_value='/bin/tool'), patch('p1401.validate.validate', return_value={'ok': True}), patch('p1401.machine_handoff.verified', return_value=profile), patch('p1401.apple.download', side_effect=RuntimeError('network')), patch.object(usb, 'check_disk') as check, patch.object(usb, 'format_disk') as fmt:
            with self.assertRaisesRegex(RuntimeError, 'network'): usb.write({}, '/missing', {}, machine_profile=profile)
            check.assert_not_called(); fmt.assert_not_called()

    def test_confirmation_mismatch_cannot_write(self):
        from p1401 import linux
        with patch.object(linux.platform, 'system', return_value='Linux'), patch.object(linux.platform, 'machine', return_value='x86_64'), patch.object(linux.os, 'geteuid', return_value=0), patch.object(usb, 'list_disks') as disks:
            self.assertEqual(linux.main(['write', '/dev/sdz', '/build', '--profile', '/report', '--darwin', '24', '--confirm-erase', '/dev/sdy']), 1)
            disks.assert_not_called()


if __name__ == '__main__': unittest.main()
