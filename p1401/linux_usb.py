"""Linux USB backend. No shell interpolation; unknown disk state fails closed."""
import json
import os
from pathlib import Path
import shutil
import stat
import subprocess
import tempfile
import time

from .usbwriter import UsbError, eligible, PART_CAP, verify_efi_copy, needs_nullmoth, _fetch

# Refuse OS volumes, including inactive installations on USB disks.
OS_FS = {'ext2', 'ext3', 'ext4', 'btrfs', 'xfs', 'zfs_member', 'linux_raid_member',
         'LVM2_member', 'crypto_LUKS', 'swap', 'apfs', 'hfs', 'hfsplus', 'ntfs', 'BitLocker'}
OS_PART = {'c12a7328-f81f-11d2-ba4b-00a0c93ec93b', '7c3457ef-0000-11aa-aa11-00306543ecac',
           '48465300-0000-11aa-aa11-00306543ecac', 'e3c9e316-0b5c-4db8-817d-f92df00215ae',
           'de94bba4-06d1-4d40-a16a-bfd50179d6ac', '0x82', '0x83', '0x8e'}


def run(*args):
    try:
        return subprocess.run(args, check=True, capture_output=True, text=True, timeout=300).stdout
    except (OSError, subprocess.SubprocessError) as error:
        detail = getattr(error, 'stderr', '') or str(error)
        raise UsbError(f'{args[0]} failed: {detail[-1500:]}') from error


def descendants(node):
    yield node
    for child in node.get('children', []):
        yield from descendants(child)


def describe(node):
    nodes = list(descendants(node))
    mounts = [m for n in nodes for m in (n.get('mountpoints') or []) if m]
    unsafe = any(n.get('fstype') in OS_FS or str(n.get('parttype') or '').lower() in OS_PART for n in nodes)
    # Allow only ordinary removable-media mounts. This also protects /, /boot,
    # /home, bind mounts, active swap and disks backing mounted device-mapper volumes.
    protected = any(not (m.startswith('/media/') or m.startswith('/run/media/') or m.startswith('/mnt/')) for m in mounts)
    busy = any(n.get('type') not in ('disk', 'part') for n in nodes)
    return {'Path': node['path'], 'FriendlyName': (node.get('model') or '').strip(),
            'SerialNumber': (node.get('serial') or '').strip(), 'Size': int(node.get('size') or 0),
            'BusType': 'USB' if node.get('tran') == 'usb' else node.get('tran'),
            'IsBoot': protected, 'IsSystem': unsafe or busy,
            'IsOffline': False, 'IsReadOnly': bool(node.get('ro')),
            'DeviceId': node.get('maj:min'), 'Nodes': nodes, 'Mounts': mounts}


def list_disks():
    data = json.loads(run('lsblk', '--json', '--bytes', '--paths', '--output',
                          'PATH,TYPE,SIZE,MODEL,SERIAL,TRAN,RO,FSTYPE,PARTTYPE,MOUNTPOINTS,MAJ:MIN'))
    return [describe(n) for n in data['blockdevices'] if n.get('type') == 'disk']


def check_disk(selected, allow_large=False):
    path = selected['Path']
    if os.path.realpath(path) != path or not path.startswith('/dev/'):
        raise UsbError('Select a canonical whole-disk /dev path, not a symlink.')
    current = next((d for d in list_disks() if d['Path'] == path), None)
    keys = ('Path', 'SerialNumber', 'FriendlyName', 'Size', 'DeviceId', 'BusType')
    if not current or any(current[k] != selected[k] for k in keys):
        raise UsbError('The USB disk changed since selection. Nothing was erased.')
    if not current['SerialNumber']:
        raise UsbError('The USB disk has no stable serial number; automatic erasure refused.')
    ok, why = eligible(current, allow_large)
    if not ok:
        raise UsbError('refusing: ' + '; '.join(why))
    st = os.stat(path)
    if not stat.S_ISBLK(st.st_mode) or f'{os.major(st.st_rdev)}:{os.minor(st.st_rdev)}' != current['DeviceId']:
        raise UsbError('The device node no longer matches the selected disk.')
    for n in current['Nodes']:
        holders = Path('/sys/dev/block', n['maj:min'], 'holders')
        if not holders.is_dir() or any(holders.iterdir()):
            raise UsbError('Disk is in use or its kernel state is unavailable.')
    return current


def format_disk(path, size, recheck=None):
    """Internal formatter. Caller must hold and revalidate the selected USB disk."""
    sectors = min(size - 64 * (1 << 20), PART_CAP) // 512
    # sfdisk specifies sizes in sectors; MBR type EF makes the FAT32 volume UEFI bootable.
    script = f'label: dos\nunit: sectors\n\nstart=2048, size={sectors}, type=ef, bootable\n'
    fd = os.open(path, os.O_RDONLY | os.O_EXCL | os.O_NOFOLLOW)
    try:
        if recheck: recheck()
        # The exclusive claim itself makes BLKRRPART report busy. Keep the claim
        # during the destructive write, then release it before the kernel reread.
        subprocess.run(['sfdisk', '--no-reread', '--no-tell-kernel', '--wipe', 'always', '--wipe-partitions', 'always', path],
                       input=script, text=True, capture_output=True, check=True, timeout=300)
    except subprocess.SubprocessError as error:
        raise UsbError('Partitioning failed: ' + (getattr(error, 'stderr', '') or str(error))) from error
    finally:
        os.close(fd)
    run('blockdev', '--rereadpt', path)
    run('udevadm', 'settle')
    partition = path + ('p1' if path[-1].isdigit() else '1')
    for _ in range(40):
        if os.path.exists(partition):
            break
        time.sleep(.25)
    if not os.path.exists(partition):
        raise UsbError('The new USB partition did not appear.')
    if recheck:
        state = recheck()
        if state['Mounts']:
            raise UsbError('The new partition was mounted by another process; formatting stopped.')
    fd = os.open(partition, os.O_RDONLY | os.O_EXCL | os.O_NOFOLLOW)
    try:
        run('mkfs.fat', '-F', '32', '-n', '1401', partition)
    finally:
        os.close(fd)
    return partition


def write(disk, efi_build_dir, recovery_info, allow_large=False, progress=None, machine_profile=None):
    from . import apple, validate, machine_handoff, nullmoth
    if os.geteuid() != 0:
        raise UsbError('USB writing requires root. Run sudo 1401 write ...')
    for tool in ('lsblk', 'sfdisk', 'mkfs.fat', 'mount', 'umount', 'udevadm', 'blockdev'):
        if not shutil.which(tool):
            raise UsbError(f'Missing system tool: {tool}')
    checked = validate.validate(efi_build_dir)
    if not checked.get('ok'):
        raise UsbError('The EFI must pass validation before erasing: ' + str(checked))
    if machine_profile is None or machine_handoff.verified(efi_build_dir) != machine_profile:
        raise UsbError('A verified build profile is required before erasing.')
    # Fetch and verify recovery before touching storage. Network failure cannot erase a stick.
    with tempfile.TemporaryDirectory(prefix='1401-recovery-') as staging:
        got = apple.download(recovery_info, staging, progress)
        driver = None
        if needs_nullmoth(efi_build_dir):
            cache = os.path.join(efi_build_dir, '.cache')
            os.makedirs(cache, exist_ok=True)
            driver = nullmoth.stage(staging, cache, fetch=_fetch)
        current = check_disk(disk, allow_large)
        # Exclusive block-device open prevents erasing disks mounted by another process.
        for mount in sorted(set(current['Mounts']), key=len, reverse=True):
            run('umount', '--', mount)
        current = check_disk(disk, allow_large)
        partition = format_disk(disk['Path'], current['Size'],
                                recheck=lambda: check_disk(disk, allow_large))
        with tempfile.TemporaryDirectory(prefix='1401-usb-') as root:
            run('mount', '-t', 'vfat', '-o', 'nodev,nosuid,noexec', partition, root)
            try:
                shutil.copytree(os.path.join(staging, 'com.apple.recovery.boot'), os.path.join(root, 'com.apple.recovery.boot'))
                shutil.copytree(os.path.join(efi_build_dir, 'EFI'), os.path.join(root, 'EFI'))
                if driver:
                    shutil.copytree(os.path.join(staging, 'NullMoth'), os.path.join(root, 'NullMoth'))
                machine_handoff.copy_to_installer(machine_profile, root)
                os.sync()
                verify_efi_copy(os.path.join(efi_build_dir, 'EFI'), os.path.join(root, 'EFI'))
                verify_efi_copy(os.path.join(staging, 'com.apple.recovery.boot'), os.path.join(root, 'com.apple.recovery.boot'))
                if driver:
                    verify_efi_copy(os.path.join(staging, 'NullMoth'), os.path.join(root, 'NullMoth'))
            finally:
                run('umount', '--', root)
    return {'device': disk['Path'], 'partition': partition, 'nullmoth': bool(driver),
            'bytes': got['bytes'], 'chunks': got['chunks']}
