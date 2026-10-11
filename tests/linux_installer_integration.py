"""Actions-only live installer test. No physical device or Apple image is published.

Only check_disk is adapted to an exclusively owned loop file. The installed
writer, Apple signature checks, formatter, copy and readback code stay real.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import plistlib
import re
import shutil
import socket
import stat
import subprocess
import tempfile
import time
from unittest.mock import patch

from p1401 import apple, linux_usb as usb, linux_provenance, machine_handoff


def owned(state):
    loop, image = state['disk']['Path'], Path(state['image'])
    assert re.fullmatch(r'/dev/loop\d+', loop), 'Only loop devices are allowed in this test'
    assert image.is_file() and not image.is_symlink()
    rows = json.loads(usb.run('losetup', '--json', '--output', 'NAME,BACK-FILE', loop))['loopdevices']
    assert len(rows) == 1 and Path(rows[0]['back-file']).resolve() == image.resolve(), 'Loop backing file changed'
    node = os.stat(loop)
    assert stat.S_ISBLK(node.st_mode)
    assert f'{os.major(node.st_rdev)}:{os.minor(node.st_rdev)}' == state['disk']['DeviceId']
    assert image.stat().st_size == state['disk']['Size']
    return dict(state['disk'], Mounts=[])


def setup(path):
    assert not path.exists()
    version = usb.run('qemu-system-x86_64', '--version').splitlines()[0]
    match = re.search(r'version (\d+)\.(\d+)', version)
    assert match and tuple(map(int, match.groups())) >= (7, 2), 'QEMU >= 7.2 required for HfsPlus AVX initialization'
    image = path.with_suffix('.img')
    with image.open('xb') as stream: stream.truncate(8 << 30)
    loop = usb.run('losetup', '--find', '--show', '--partscan', str(image)).strip()
    node = os.stat(loop)
    disk = {'Path': loop, 'FriendlyName': 'CI virtual USB — disposable loop image',
            'SerialNumber': 'CI-loop-fixture', 'Size': image.stat().st_size,
            'DeviceId': f'{os.major(node.st_rdev)}:{os.minor(node.st_rdev)}',
            'BusType': 'loop', 'IsBoot': False, 'IsSystem': False, 'IsOffline': False,
            'IsReadOnly': False, 'Mounts': [], 'Nodes': [], 'eligible': True, 'reasons': []}
    path.write_text(json.dumps({'image': str(image), 'disk': disk, 'qemu_version': version}))
    print('Allocated disposable loop image:', loop, flush=True)


def boot(image, output):
    from PIL import Image
    pairs = [(Path('/usr/share/OVMF') / f'OVMF_CODE{suffix}.fd', Path('/usr/share/OVMF') / f'OVMF_VARS{suffix}.fd')
             for suffix in ('', '_4M')]
    code, variables = next((c, v) for c, v in pairs if c.is_file() and v.is_file())
    with tempfile.TemporaryDirectory(prefix='1401-qemu-') as td:
        root = Path(td); shutil.copy2(variables, root / 'vars.fd')
        serial = output / 'uefi-serial.txt'
        args = ['qemu-system-x86_64', '-machine', 'q35,accel=tcg', '-cpu', 'max', '-m', '1024',
                '-nodefaults', '-device', 'VGA', '-display', 'none', '-no-reboot',
                '-drive', f'if=pflash,format=raw,unit=0,readonly=on,file={code}',
                '-drive', f'if=pflash,format=raw,unit=1,file={root / "vars.fd"}',
                '-device', 'qemu-xhci,id=xhci', '-device', 'usb-kbd',
                '-drive', f'if=none,id=stick,format=raw,snapshot=on,file={image}',
                '-device', 'usb-storage,drive=stick,bootindex=1,removable=on',
                '-serial', f'file:{serial}', '-qmp', f'unix:{root / "qmp.sock"},server=on,wait=off']
        with (output / 'qemu-stderr.txt').open('w') as err:
            process = subprocess.Popen(args, stdout=subprocess.DEVNULL, stderr=err)
            client = None
            try:
                deadline = time.monotonic() + 150
                while not (root / 'qmp.sock').exists():
                    assert process.poll() is None, 'QEMU exited before firmware started'
                    assert time.monotonic() < deadline; time.sleep(.1)
                client = socket.socket(socket.AF_UNIX); client.settimeout(10); client.connect(str(root / 'qmp.sock'))
                stream = client.makefile('rwb'); greeting = json.loads(stream.readline()); assert 'QMP' in greeting
                def qmp(command, arguments=None):
                    stream.write((json.dumps({'execute': command, 'arguments': arguments or {}}) + '\n').encode()); stream.flush()
                    while True:
                        reply = json.loads(stream.readline())
                        if 'error' in reply: raise RuntimeError(str(reply))
                        if 'return' in reply: return reply['return']
                qmp('qmp_capabilities')
                text = ''
                while time.monotonic() < deadline:
                    assert process.poll() is None, 'QEMU exited during boot'
                    qmp('screendump', {'filename': str(root / 'screen.ppm')})
                    with Image.open(root / 'screen.ppm') as frame: frame.save(output / '09-uefi-picker.png')
                    text = subprocess.run(['tesseract', str(output / '09-uefi-picker.png'), 'stdout', '--psm', '6'],
                                          capture_output=True, text=True, check=True, timeout=15).stdout
                    if re.search(r'(mac\s*os.*base\s*system|recovery)', text, re.I):
                        (output / 'uefi-screen-text.txt').write_text(text)
                        print('OVMF booted the USB image into the OpenCore recovery picker.', flush=True)
                        return {'firmware': code.name, 'emulation': 'QEMU TCG', 'recovery_entry_visible': True,
                                'macos_kernel_boot': 'not tested', 'physical_usb': 'not tested'}
                    time.sleep(2)
                (output / 'uefi-screen-text.txt').write_text(text)
                raise AssertionError('Recovery entry did not appear in the UEFI screenshot; inspect evidence')
            finally:
                if client: client.close()
                process.terminate()
                try: process.wait(timeout=10)
                except subprocess.TimeoutExpired: process.kill(); process.wait()


def write(state, build, report, output):
    output.mkdir(parents=True, exist_ok=True)
    result = {'schema': 1, 'ok': False, 'source': 'pinned corpus; not the user laptop',
              'recovery_source': 'Apple directly; no recovery image retained as an artifact',
              'qemu_version': state['qemu_version']}
    try:
        owned(state)
        # Negative control: production USB selection must refuse this loop.
        try: usb.check_disk(state['disk'])
        except usb.UsbError: result['production_loop_refused'] = True
        else: raise AssertionError('Production USB checker accepted a loop device')
        with tempfile.TemporaryDirectory(prefix='1401-ci-build-') as td:
            copied = Path(td) / 'build'; shutil.copytree(build, copied)
            cfg = copied / 'EFI/OC/config.plist'
            with cfg.open('rb') as stream: config = plistlib.load(stream)
            # Hold the picker for evidence; hardware settings are not changed.
            changes = {'ShowPicker': True, 'Timeout': 0, 'PickerMode': 'Builtin'}
            config['Misc']['Boot'].update(changes)
            config['Misc']['Debug']['Target'] |= 0x9
            with cfg.open('wb') as stream: plistlib.dump(config, stream)
            profile = machine_handoff.verified(build, report)
            profile['efi_sha256'] = machine_handoff.efi_hashes(copied)
            (copied / machine_handoff.NAME).write_text(json.dumps(profile))
            result['ci_only_settings'] = {**changes, 'Debug.Target': config['Misc']['Debug']['Target']}
            info = apple.image_info(24)
            result['apple_product'] = info['product']; result['apple_board'] = info['board']
            last = [-1]
            def progress(done, total):
                percent = done * 100 // total
                if percent != last[0]: print(f'Download {done}/{total}', flush=True); last[0] = percent
            def checked(selected, allow_large=False):
                assert selected['Path'] == state['disk']['Path']; return owned(state)
            with linux_provenance.for_profile(profile), patch.object(usb, 'check_disk', side_effect=checked):
                got = usb.write(state['disk'], str(copied), info, progress=progress, machine_profile=profile)
            print('USB write completed. Independently checking remounted EFI and all signed recovery chunks.', flush=True)
            result['writer'] = got
            with tempfile.TemporaryDirectory(prefix='1401-readback-') as mount:
                usb.run('mount', '-t', 'vfat', '-o', 'ro,nodev,nosuid,noexec', got['partition'], mount)
                try:
                    root = Path(mount)
                    usb.verify_efi_copy(str(copied / 'EFI'), str(root / 'EFI'))
                    recovery = root / 'com.apple.recovery.boot'
                    lists = list(recovery.glob('*.chunklist')); assert len(lists) == 1
                    chunks = apple.parse_chunklist(lists[0].read_bytes())
                    images = list(recovery.glob('*.dmg')); assert len(images) == 1
                    with images[0].open('rb') as stream:
                        for size, expected in chunks: assert hashlib.sha256(apple._read_exact(stream, size)).digest() == expected
                        assert not stream.read(1)
                    result['readback'] = {'efi': True, 'apple_signature': True, 'apple_chunks': len(chunks),
                                          'bootx64': (root / 'EFI/BOOT/BOOTx64.efi').is_file()}
                    assert result['readback']['bootx64']
                    result['files'] = {str(p.relative_to(root)): machine_handoff.sha256(p)
                                       for p in root.rglob('*') if p.is_file()}
                finally: usb.run('umount', mount)
            result['partition_table'] = json.loads(usb.run('sfdisk', '--json', state['disk']['Path']))['partitiontable']
            partition = result['partition_table']['partitions'][0]
            assert partition['type'].lower() == 'ef' and partition['bootable']
            usb.run('losetup', '--detach', state['disk']['Path'])
            print('Readback passed. Booting the image with OVMF and AVX-capable QEMU TCG.', flush=True)
            result['boot'] = boot(state['image'], output)
        result['ok'] = True
    except Exception as error:
        result['error'] = f'{type(error).__name__}: {error}'; raise
    finally:
        (output / 'installer-test.json').write_text(json.dumps(result, indent=2))


def main():
    ap = argparse.ArgumentParser(); ap.add_argument('action', choices=['setup', 'write', 'cleanup'])
    ap.add_argument('--state', required=True); ap.add_argument('--build'); ap.add_argument('--report'); ap.add_argument('--output')
    args = ap.parse_args()
    assert os.geteuid() == 0 and os.environ.get('GITHUB_ACTIONS') == 'true', 'This is an Actions-only test adapter'
    state_path = Path(args.state).resolve()
    if args.action == 'setup': setup(state_path); return
    state = json.loads(state_path.read_text())
    if args.action == 'write': write(state, args.build, args.report, Path(args.output).resolve())
    else:
        if usb.run('losetup', '--associated', state['image']).strip():
            owned(state); usb.run('losetup', '--detach', state['disk']['Path'])
        Path(state['image']).unlink(); state_path.unlink()


if __name__ == '__main__': main()
