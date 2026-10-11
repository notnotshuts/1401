import unittest
from p1401 import linux_provenance as lp, validate


class LinuxProvenance(unittest.TestCase):
    def test_root_validation_uses_recorded_official_release_and_restores_state(self):
        release = {'url': 'https://github.com/dortania/build-repo/releases/download/example/OpenCore.zip', 'sha256': 'a' * 64}
        original = validate._opencore_release
        with self.assertRaisesRegex(RuntimeError, 'fixture'):
            with lp.for_profile({'linux_opencore_release': release}):
                self.assertEqual(validate._opencore_release(), (release['url'], release['sha256']))
                raise RuntimeError('fixture')
        self.assertIs(validate._opencore_release, original)

    def test_untrusted_or_missing_provenance_never_selects_executable(self):
        original = validate._opencore_release
        for url in ('file:///tmp/evil.zip', 'https://github.com/attacker/releases/evil.zip', 'https://github.com.evil/acidanthera/OpenCorePkg/releases/download/a.zip'):
            with self.subTest(url=url), self.assertRaises(RuntimeError):
                with lp.for_profile({'linux_opencore_release': {'url': url, 'sha256': 'a' * 64}}): self.fail('entered')
        self.assertIs(validate._opencore_release, original)


if __name__ == '__main__': unittest.main()
