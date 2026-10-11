"""CI-only formatter integration: a disposable sparse file, never a physical disk."""
import os
import json
from pathlib import Path
import tempfile
from unittest.mock import patch
from p1401 import linux_usb as usb

with tempfile.TemporaryDirectory() as td:
    image = Path(td) / 'disk.img'
    with image.open('wb') as f: f.truncate(3 << 30)
    loop = usb.run('losetup', '--find', '--show', '--partscan', str(image)).strip()
    try:
        partition = usb.format_disk(loop, 3 << 30)
        root = Path(td) / 'mount'; root.mkdir()
        usb.mount_partition(partition, str(root))
        try:
            actual_run = usb.run
            def inventory(*args):
                value = actual_run(*args)
                if args[0] == 'lsblk':
                    data = json.loads(value)
                    # Discovery-only adapter: include our loop in the disk list.
                    # Partition and mount metadata remain the actual kernel state.
                    for node in data['blockdevices']:
                        if node['path'] == loop: node['type'] = 'disk'
                    return json.dumps(data)
                return value
            with patch.object(usb, 'run', side_effect=inventory):
                found = next(d for d in usb.list_disks() if d['Path'] == loop)
            assert found['Partitions'] == [partition], found
            assert str(root) in found['Mounts'], found
            assert found['IsBoot'], 'A protected mount must prevent erasure'
            (root / 'EFI/BOOT').mkdir(parents=True)
            (root / 'EFI/BOOT/BOOTx64.efi').write_bytes(b'fixture')
            os.sync()
            assert (root / 'EFI/BOOT/BOOTx64.efi').read_bytes() == b'fixture'
        finally: usb.run('umount', str(root))
        print('Kernel partition identity, mounted-child discovery, checked FAT32 mount, write and readback passed.')
    finally: usb.run('losetup', '--detach', loop)
