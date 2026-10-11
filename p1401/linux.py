"""Linux desktop and command-line entry point, using the shared 1401 engine."""
import argparse
import json
import os
from pathlib import Path
import platform
import subprocess
import sys

REPO = Path(__file__).resolve().parent.parent


def child(*args):
    result = subprocess.run([sys.executable, '-u', '-m', *args], cwd=REPO)
    if result.returncode:
        raise RuntimeError(f'{args[0]} failed (exit {result.returncode})')


def make_guide(report_path, build, output):
    from . import guide, machine_handoff, linux_guide
    rep, cfg = guide.load(report_path, str(Path(build) / 'EFI/OC/config.plist'))
    profile = machine_handoff.verified(build, report_path)
    Path(output).write_text(linux_guide.render(rep, cfg, profile), encoding='utf-8')
    print(f'BIOS guide: {output}')


def parser():
    ap = argparse.ArgumentParser(prog='1401', description='1401 Linux: scan hardware, build EFI, create an installer USB')
    sub = ap.add_subparsers(dest='command')
    sub.add_parser('gui', help='open the Linux desktop interface')
    p = sub.add_parser('scan', help='collect hardware and ACPI (requires root)')
    p.add_argument('output')
    for name in ('plan', 'build'):
        p = sub.add_parser(name)
        p.add_argument('report'); p.add_argument('acpi')
        if name == 'build': p.add_argument('output')
        p.add_argument('--macos', default='')
        if name == 'build': p.add_argument('--result', help='save structured build results')
    p = sub.add_parser('check', help='check hardware compatibility using OpCore-Simplify rules')
    p.add_argument('report'); p.add_argument('--macos', type=int, default=24, choices=range(19, 26))
    p.add_argument('--output')
    sub.add_parser('disks', help='list disks and whether they can be erased')
    p = sub.add_parser('guide'); p.add_argument('report'); p.add_argument('build'); p.add_argument('output')
    p = sub.add_parser('write', help='ERASE a USB disk and create a macOS installer (requires root)')
    p.add_argument('device'); p.add_argument('build'); p.add_argument('--profile', required=True)
    p.add_argument('--darwin', type=int, required=True, choices=range(19, 26))
    p.add_argument('--confirm-erase', required=True, metavar='/dev/sdX', help='repeat the exact whole-disk device path')
    p.add_argument('--allow-large', action='store_true')
    return ap


def main(argv=None):
    ap = parser(); a = ap.parse_args(argv)
    if platform.system() != 'Linux' or platform.machine() != 'x86_64':
        ap.error('This port supports x86_64 Linux.')
    try:
        if a.command in (None, 'gui'):
            gui(); return 0
        if a.command == 'scan':
            if os.geteuid() != 0: raise RuntimeError('Run sudo 1401 scan OUTPUT to read ACPI tables.')
            child('p1401.scan', str(Path(a.output).resolve()))
        elif a.command == 'check':
            from . import linux_hardware
            result = linux_hardware.analyze(a.report, a.macos)
            if a.output: linux_hardware.save(result, a.output)
            print(json.dumps(result, indent=2, ensure_ascii=False))
            return 0 if result['ok'] else 1
        elif a.command in ('plan', 'build'):
            args = [a.command, str(Path(a.report).resolve()), str(Path(a.acpi).resolve())]
            if a.command == 'build': args.append(str(Path(a.output).resolve()))
            if a.command == 'build' and a.result:
                args.extend(['--result', str(Path(a.result).resolve())])
            child('p1401', *args, '--macos', a.macos, '--echo', '--json')
            if a.command == 'build':
                from . import linux_provenance
                linux_provenance.record(str(Path(a.output).resolve()), str(Path(a.report).resolve()))
        elif a.command == 'guide':
            make_guide(a.report, a.build, a.output)
        elif a.command == 'disks':
            from . import linux_usb
            for d in linux_usb.list_disks():
                ok, reasons = linux_usb.eligible(d)
                print(json.dumps({k: v for k, v in d.items() if k not in ('Nodes', 'Mounts')} | {'eligible': ok, 'reasons': reasons}))
        elif a.command == 'write':
            from . import linux_usb, apple, machine_handoff, tls
            if os.geteuid() != 0: raise RuntimeError('Run sudo 1401 write ...')
            if a.device != a.confirm_erase: raise RuntimeError('--confirm-erase must repeat the selected device path.')
            disk = next((d for d in linux_usb.list_disks() if d['Path'] == a.device), None)
            if disk is None: raise RuntimeError('Whole USB disk not found.')
            linux_usb.check_disk(disk, a.allow_large)
            profile = machine_handoff.verified(a.build, a.profile)
            tls.install()
            from . import linux_provenance
            with linux_provenance.for_profile(profile):
                result = linux_usb.write(disk, a.build, apple.image_info(a.darwin), a.allow_large,
                                         progress=lambda done, total, *_: print(f'Download {done}/{total}', flush=True),
                                         machine_profile=profile)
            print('Installer USB ready:', json.dumps(result, default=str))
        return 0
    except Exception as error:
        print(f'1401: {error}', file=sys.stderr)
        return 1


def gui():
    from .linux_gui import Application
    Application().mainloop()


if __name__ == '__main__':
    sys.exit(main())
