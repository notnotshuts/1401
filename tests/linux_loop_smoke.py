"""CI-only formatter integration: a disposable sparse file, never a physical disk."""
import os
from pathlib import Path
import tempfile
from p1401 import linux_usb as usb

with tempfile.TemporaryDirectory() as td:
    image = Path(td) / 'disk.img'
    with image.open('wb') as f: f.truncate(3 << 30)
    loop = usb.run('losetup', '--find', '--show', '--partscan', str(image)).strip()
    try:
        partition = usb.format_disk(loop, 3 << 30)
        root = Path(td) / 'mount'; root.mkdir()
        usb.run('mount', '-t', 'vfat', '-o', 'nodev,nosuid,noexec', partition, str(root))
        try:
            (root / 'EFI/BOOT').mkdir(parents=True)
            (root / 'EFI/BOOT/BOOTx64.efi').write_bytes(b'fixture')
            os.sync()
            assert (root / 'EFI/BOOT/BOOTx64.efi').read_bytes() == b'fixture'
        finally: usb.run('umount', str(root))
        print('Loop-device partition, FAT32 format, mount, write and readback passed.')
    finally: usb.run('losetup', '--detach', loop)
