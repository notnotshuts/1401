import hashlib
from pathlib import Path
import tempfile
import unittest
from tools.linux_release_assets import prepare


class ReleaseAssets(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(); self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name); self.source = self.root / 'built'; self.source.mkdir()
        (self.source / 'ui').mkdir()
        for name in ('p1401-linux_test_amd64.deb', '1401-linux-x86_64-test.tar.gz'):
            (self.source / name).write_bytes(name.encode())
        (self.source / 'SHA256SUMS').write_text(''.join(f'{hashlib.sha256(p.read_bytes()).hexdigest()}  {p.name}\n' for p in self.source.iterdir() if p.is_file()))
        for name in ('hardware.png', 'overview.png'): (self.source / 'ui' / name).write_bytes(b'png')
        self.repo = self.root / 'repo'; (self.repo / 'docs').mkdir(parents=True)
        for name in ('README.md', 'LICENSE', 'NOTICE.md', 'docs/LINUX.md'): (self.repo / name).write_text('fixture')
        self.destination = self.root / 'release'

    def test_every_published_file_has_a_checksum(self):
        names = prepare(self.source, self.destination, self.repo)
        self.assertEqual(len(names), 9)
        for line in (self.destination / 'SHA256SUMS').read_text().splitlines():
            digest, name = line.split('  ', 1)
            self.assertEqual(hashlib.sha256((self.destination / name).read_bytes()).hexdigest(), digest)
        self.assertIn('README-Linux.md', names)

    def test_changed_package_is_refused_before_staging(self):
        (self.source / 'p1401-linux_test_amd64.deb').write_bytes(b'tampered')
        with self.assertRaisesRegex(ValueError, 'checksum mismatch'): prepare(self.source, self.destination, self.repo)
        self.assertFalse(self.destination.exists())

    def test_incomplete_release_is_refused(self):
        (self.source / 'ui/hardware.png').unlink()
        with self.assertRaisesRegex(ValueError, 'missing'): prepare(self.source, self.destination, self.repo)
        self.assertFalse(self.destination.exists())


if __name__ == '__main__': unittest.main()
