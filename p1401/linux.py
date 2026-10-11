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
    from . import guide, machine_handoff
    rep, cfg = guide.load(report_path, str(Path(build) / 'EFI/OC/config.plist'))
    profile = machine_handoff.verified(build, report_path)
    steps = guide.build_guide(rep, cfg, disabled_devices=profile.get('disabled_devices', {}))
    Path(output).write_text(guide.render_text(steps), encoding='utf-8')
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
        elif a.command in ('plan', 'build'):
            args = [a.command, str(Path(a.report).resolve()), str(Path(a.acpi).resolve())]
            if a.command == 'build': args.append(str(Path(a.output).resolve()))
            child('p1401', *args, '--macos', a.macos, '--echo', '--json')
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
            result = linux_usb.write(disk, a.build, apple.image_info(a.darwin), a.allow_large,
                                     progress=lambda done, total, *_: print(f'Download {done}/{total}', flush=True),
                                     machine_profile=profile)
            print('Installer USB ready:', json.dumps(result, default=str))
        return 0
    except Exception as error:
        print(f'1401: {error}', file=sys.stderr)
        return 1


def gui():
    import tkinter as tk
    from tkinter import ttk, filedialog, messagebox
    import queue
    import threading
    root = tk.Tk(); root.title('1401 — Linux'); root.geometry('850x640')
    frame = ttk.Frame(root, padding=16); frame.pack(fill='both', expand=True)
    ttk.Label(frame, text='1401 for Linux', font=('', 20)).pack(anchor='w')
    ttk.Label(frame, text='Check hardware → Build EFI → Review BIOS guide → Create installer USB').pack(anchor='w', pady=8)
    workspace = tk.StringVar(value=str(Path.home() / '1401-work'))
    row = ttk.Frame(frame); row.pack(fill='x')
    ttk.Entry(row, textvariable=workspace).pack(side='left', fill='x', expand=True)
    ttk.Button(row, text='Workspace…', command=lambda: workspace.set(filedialog.askdirectory() or workspace.get())).pack(side='left')
    options = ttk.Frame(frame); options.pack(fill='x', pady=8)
    macos = tk.StringVar(value='24')
    ttk.Label(options, text='Darwin version (24 = macOS 15):').pack(side='left')
    ttk.Combobox(options, textvariable=macos, values=list(range(19, 26)), state='readonly', width=6).pack(side='left')
    device = tk.StringVar()
    ttk.Label(options, text=' USB device:').pack(side='left')
    ttk.Entry(options, textvariable=device, width=18).pack(side='left')
    buttons = ttk.Frame(frame); buttons.pack(fill='x', pady=8)
    log = tk.Text(frame, wrap='word'); log.pack(fill='both', expand=True)
    messages = queue.Queue(); controls = []; busy = [False]
    def task(args):
        if busy[0]: return
        busy[0] = True
        for b in controls: b.configure(state='disabled')
        def worker():
            try:
                p = subprocess.Popen([sys.executable, '-u', '-m', 'p1401.linux', *args], cwd=REPO,
                                     stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
                for line in p.stdout: messages.put(line)
                messages.put(f'Process exited: {p.wait()}\n')
            except Exception as error: messages.put(str(error) + '\n')
            finally: messages.put(None)
        threading.Thread(target=worker, daemon=True).start()
    def paths():
        w = Path(workspace.get()).expanduser().resolve(); w.mkdir(parents=True, exist_ok=True)
        return w, w / 'scan/Report.json', w / 'scan/ACPI', w / 'build'
    def scan():
        if os.geteuid() != 0:
            messagebox.showinfo('Root required', f'Run in a terminal:\nsudo 1401 scan {paths()[0] / "scan"}\nThen use Build EFI here.'); return
        task(['scan', str(paths()[0] / 'scan')])
    def build():
        w, report, acpi, output = paths()
        task(['build', str(report), str(acpi), str(output), '--macos', macos.get()])
    def guide():
        w, report, _, output = paths(); task(['guide', str(report), str(output), str(w / 'BIOS.txt')])
    def write():
        w, report, _, output = paths()
        if os.geteuid() != 0:
            messagebox.showinfo('Root required', f'Run in a terminal:\nsudo 1401 write {device.get()} {output} --profile {report} --darwin {macos.get()} --confirm-erase {device.get()}'); return
        if messagebox.askyesno('Erase USB disk?', f'ALL DATA on {device.get()} will be erased. Continue?'):
            task(['write', device.get(), str(output), '--profile', str(report), '--darwin', macos.get(), '--confirm-erase', device.get()])
    for title, fn in [('Check PC', scan), ('Build EFI', build), ('BIOS guide', guide), ('List disks', lambda: task(['disks'])), ('Create USB', write)]:
        b = ttk.Button(buttons, text=title, command=fn); b.pack(side='left', padx=3); controls.append(b)
    def poll():
        while not messages.empty():
            item = messages.get()
            if item is None:
                busy[0] = False
                for b in controls: b.configure(state='normal')
            else: log.insert('end', item); log.see('end')
        root.after(100, poll)
    def close():
        if busy[0]: messagebox.showinfo('Operation running', 'Wait for the current operation to finish before closing.'); return
        root.destroy()
    root.protocol('WM_DELETE_WINDOW', close); poll(); root.mainloop()


if __name__ == '__main__':
    sys.exit(main())
