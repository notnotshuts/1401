"""Assemble complete, checksum-verified assets before a GitHub Release is published."""
import argparse
import hashlib
from pathlib import Path
import shutil

ROOT = Path(__file__).resolve().parent.parent


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def prepare(source, destination, repo=ROOT):
    source, destination, repo = Path(source), Path(destination), Path(repo)
    # Check the package manifest from the build before preparing release files.
    manifest = (source / 'SHA256SUMS').read_text(encoding='utf-8')
    checked = set()
    for line in manifest.splitlines():
        checksum, name = line.split('  ', 1)
        if Path(name).name != name or (source / name).is_symlink():
            raise ValueError('Invalid package manifest path: ' + name)
        if digest(source / name) != checksum:
            raise ValueError('Package checksum mismatch: ' + name)
        checked.add(name)
    debs = list(source.glob('p1401-linux_*_amd64.deb'))
    archives = list(source.glob('1401-linux-x86_64-*.tar.gz'))
    if len(debs) != 1 or len(archives) != 1:
        raise ValueError('Exactly one .deb and one portable archive are required.')
    for path in debs + archives:
        if path.name not in checked:
            raise ValueError('Package is not covered by build checksums: ' + path.name)
    files = {p.name: p for p in debs + archives}
    files.update({'hardware.png': source / 'ui/hardware.png', 'overview.png': source / 'ui/overview.png',
                  'README-Linux.md': repo / 'README.md', 'INSTALL.md': repo / 'docs/LINUX.md',
                  'LICENSE': repo / 'LICENSE', 'NOTICE.md': repo / 'NOTICE.md'})
    for name, path in files.items():
        if not path.is_file() or path.is_symlink():
            raise ValueError('Required release asset missing or linked: ' + name)
    destination.mkdir(parents=True, exist_ok=True)
    if any(destination.iterdir()): raise ValueError('Release staging directory must be empty.')
    for name, path in files.items(): shutil.copy2(path, destination / name)
    # Flatten screenshots for direct downloads and keep the standalone README's
    # repository references usable outside the source checkout.
    readme = destination / 'README-Linux.md'
    content = readme.read_text(encoding='utf-8').replace('(docs/images/hardware.png)', '(hardware.png)')
    for prefix in ('docs/', '.github/'):
        content = content.replace('](' + prefix, '](https://github.com/notnotshuts/1401/blob/main/' + prefix)
    for name in ('LICENSE', 'NOTICE.md'):
        content = content.replace('](' + name + ')', '](https://github.com/notnotshuts/1401/blob/main/' + name + ')')
    readme.write_text(content, encoding='utf-8')
    (destination / 'SHA256SUMS').write_text(''.join(
        f'{digest(path)}  {path.name}\n' for path in sorted(destination.iterdir())), encoding='utf-8')
    return sorted(path.name for path in destination.iterdir())


def main():
    ap = argparse.ArgumentParser(); ap.add_argument('--input', required=True); ap.add_argument('--output', required=True)
    a = ap.parse_args()
    print('\n'.join(prepare(a.input, a.output)))


if __name__ == '__main__': main()
