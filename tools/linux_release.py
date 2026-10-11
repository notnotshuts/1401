"""Build a Linux x86_64 .deb and portable Python runtime archive."""
import argparse
import hashlib
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile

ROOT = Path(__file__).resolve().parent.parent


def main():
    ap = argparse.ArgumentParser(); ap.add_argument('--output', required=True)
    a = ap.parse_args(); output = Path(a.output).resolve(); output.mkdir(parents=True, exist_ok=True)
    revision = subprocess.check_output(['git', 'rev-parse', '--short=12', 'HEAD'], cwd=ROOT, text=True).strip()
    version = '1.1.0+linux1.' + revision
    with tempfile.TemporaryDirectory(prefix='1401-package-') as td:
        work = Path(td); portable = work / '1401-linux-x86_64'; runtime = portable / 'runtime'; runtime.mkdir(parents=True)
        for name in ('p1401', 'upstream'):
            shutil.copytree(ROOT / name, runtime / name, ignore=shutil.ignore_patterns('__pycache__', '*.pyc', 'OCK_Files', 'Results', 'iasl*'))
        for name in ('LICENSE', 'NOTICE.md'):
            shutil.copy2(ROOT / name, runtime / name)
        (runtime / 'LINUX_VERSION').write_text(version + '\n')
        (runtime / 'entry.py').write_text('from p1401.linux import main\nraise SystemExit(main())\n')
        shutil.copy2(ROOT / 'linux/1401', portable / '1401')
        shutil.copy2(ROOT / 'docs/LINUX.md', portable / 'README.md')
        subprocess.run(['/usr/bin/python3', '-m', 'compileall', '-q', str(runtime)], check=True)
        for cache in runtime.rglob('__pycache__'): shutil.rmtree(cache)
        archive = output / f'1401-linux-x86_64-{version}.tar.gz'
        with tarfile.open(archive, 'w:gz') as tf: tf.add(portable, arcname=portable.name)
        deb = work / 'deb'; (deb / 'DEBIAN').mkdir(parents=True)
        (deb / 'DEBIAN/control').write_text(f'''Package: p1401-linux
Version: {version}
Section: utils
Priority: optional
Architecture: amd64
Maintainer: 1401 Linux fork contributors
Depends: python3 (>= 3.10), python3-tk, python3-certifi, pciutils, usbutils, util-linux, fdisk, dosfstools, udev, sudo
Description: Linux interface for the 1401 OpenCore installer builder
 Local hardware scan, EFI build, BIOS guide and guarded USB creation.
''')
        (deb / 'usr/lib').mkdir(parents=True); shutil.copytree(runtime, deb / 'usr/lib/1401')
        (deb / 'usr/bin').mkdir(parents=True); shutil.copy2(ROOT / 'linux/1401', deb / 'usr/bin/1401')
        (deb / 'usr/share/applications').mkdir(parents=True)
        shutil.copy2(ROOT / 'linux/1401.desktop', deb / 'usr/share/applications/1401.desktop')
        doc = deb / 'usr/share/doc/p1401-linux'; doc.mkdir(parents=True)
        for name in ('LICENSE', 'NOTICE.md'): shutil.copy2(ROOT / name, doc / name)
        shutil.copy2(ROOT / 'docs/LINUX.md', doc / 'README.md')
        subprocess.run(['dpkg-deb', '--root-owner-group', '--build', str(deb), str(output / f'p1401-linux_{version}_amd64.deb')], check=True)
    files = sorted(output.glob('*'))
    (output / 'SHA256SUMS').write_text(''.join(f'{hashlib.sha256(p.read_bytes()).hexdigest()}  {p.name}\n' for p in files if p.name != 'SHA256SUMS'))


if __name__ == '__main__': main()
