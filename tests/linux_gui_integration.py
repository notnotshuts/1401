"""Exercise real GUI child processes: JSON import/check -> automatic EFI -> personal guide."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
from unittest.mock import patch

ap = argparse.ArgumentParser(); ap.add_argument('--report', required=True)
ap.add_argument('--output'); ap.add_argument('--usb-test', action='store_true')
a = ap.parse_args()
output = Path(a.output).resolve() if a.output else None
if output: output.mkdir(parents=True, exist_ok=True)
with tempfile.TemporaryDirectory() as td:
    os.environ['XDG_CONFIG_HOME'] = td
    from p1401.linux_gui import Application
    app = Application(); app.workspace.set(str(Path(td) / 'workspace'))
    def capture(name):
        if not output: return
        app.update()
        subprocess.run(['scrot', '-o', str(output / name)], check=True)
    capture('01-overview.png')
    imported = Path(td) / 'Imported-Report.json'
    imported.write_text('\ufeff' + Path(a.report).read_text(), encoding='utf-8')
    app.report_path.set(str(imported)); app.acpi_path.set(str(Path(a.report).parent / 'ACPI'))
    def wait(timeout=240, progress_capture=False):
        deadline = time.monotonic() + timeout
        captured = False
        while app.busy and time.monotonic() < deadline:
            app.update(); time.sleep(.02)
            if progress_capture and not captured and 'Download ' in app.log.get('1.0', 'end'):
                capture('07-usb-progress.png'); captured = True
        assert not app.busy, 'GUI child operation timed out'
        # Flush completion callbacks before observing the state.
        app.update()
    app.check(); wait()
    assert app.analysis_current() and app.analysis['ok'], app.log.get('1.0', 'end')
    app.show('hardware'); capture('02-hardware.png')
    # Check twice: a failed/stale prior output must never win.
    app.check(); wait()
    assert app.analysis_current() and app.analysis['ok'], app.log.get('1.0', 'end')
    app.prepare(); wait()
    assert app.build_ready(), app.log.get('1.0', 'end')
    assert (app.work / 'BIOS.txt').is_file(), app.log.get('1.0', 'end')
    assert '5. Установите macOS' in app.guide_text.get('1.0', 'end')
    assert app.current_page == 'guide'
    assert not app.wizard
    app.show('build'); capture('03-efi-result.png')
    app.show('guide'); capture('04-bios-guide.png')
    # A fresh run can reuse a valid build and regenerate its guide without scanning or erasing.
    app.prepare(); wait()
    assert app.current_page == 'guide' and app.build_ready()
    # A malformed import is displayed and disables build, then a correct import recovers.
    bad = Path(td) / 'bad.json'; bad.write_text('broken')
    app.report_path.set(str(bad)); app.check(); wait()
    assert app.analysis and not app.analysis['ok']
    assert app.build_button.cget('state') == 'disabled'
    app.show('hardware'); capture('05-json-error.png')
    app.report_path.set(a.report); app.check(); wait()
    assert app.analysis_current() and app.analysis['ok']
    if a.usb_test:
        assert output and os.environ.get('GITHUB_ACTIONS') == 'true'
        app.report_path.set(str(imported)); app.check(); wait()
        assert app.build_ready(), app.log.get('1.0', 'end')
        # Tests-only device/privilege adapter; the shipped application cannot
        # select a loop device. Writer and Apple recovery remain actual code.
        helper = Path(__file__).resolve().with_name('linux_installer_integration.py')
        state = Path(td) / 'loop-state.json'
        prefix = ['sudo', '-n', 'env', 'GITHUB_ACTIONS=true',
                  'PYTHONPATH=/usr/lib/1401:/usr/lib/1401/vendor', '/usr/bin/python3', '-u', str(helper)]
        subprocess.run(prefix + ['setup', '--state', str(state)], check=True)
        try:
            disk = json.loads(state.read_text())['disk']
            label = disk['Path'] + ' · CI virtual USB (test adapter)'
            app.disks = {label: disk}; app.disk_picker.configure(values=[label]); app.disk.set(label)
            app.show('usb'); app.select_disk(); capture('06-usb-selected.png')
            from p1401 import linux_gui
            constructor = linux_gui.ctk.CTkInputDialog
            def confirmation(*args, **kwargs):
                dialog = constructor(*args, **kwargs)
                def accept():
                    dialog._entry.insert(0, disk['Path'])
                    capture('06-usb-confirmation.png')
                    dialog._ok_event()
                dialog.after(300, accept)
                return dialog
            original_command = app.command
            def command(args, privileged=False):
                if args[0] != 'write': return original_command(args, privileged)
                assert args[1] == disk['Path'] and args[-1] == disk['Path']
                return prefix + ['write', '--state', str(state), '--build', str(app.build_dir),
                                 '--report', app.report_file, '--output', str(output)]
            with patch.object(app, 'command', side_effect=command), \
                 patch.object(linux_gui.ctk, 'CTkInputDialog', side_effect=confirmation), \
                 patch.object(linux_gui.messagebox, 'showinfo') as success:
                app.write(); wait(900, progress_capture=True)
                assert success.called, app.log.get('1.0', 'end')
            result = json.loads((output / 'installer-test.json').read_text())
            assert result['ok'], result
            capture('08-usb-complete.png')
            app.show('logs'); capture('10-operation-log.png')
        finally:
            (output / 'gui-operation-log.txt').write_text(app.log.get('1.0', 'end'))
            subprocess.run(prefix + ['cleanup', '--state', str(state)], check=True)
    if output:
        (output / 'BIOS.txt').write_text((app.work / 'BIOS.txt').read_text())
        (output / 'build-result.json').write_text((app.work / 'build-result.json').read_text())
        (output / 'compatibility.json').write_text(json.dumps(app.analysis, indent=2))
    app.destroy()
print('Real GUI subprocesses, repeated JSON checks, automatic EFI/guide, reuse and error recovery passed.')
