"""Exercise real GUI child processes: JSON import/check -> automatic EFI -> personal guide."""
import argparse
import json
import os
from pathlib import Path
import tempfile
import time

ap = argparse.ArgumentParser(); ap.add_argument('--report', required=True)
a = ap.parse_args()
with tempfile.TemporaryDirectory() as td:
    os.environ['XDG_CONFIG_HOME'] = td
    from p1401.linux_gui import Application
    app = Application(); app.workspace.set(str(Path(td) / 'workspace'))
    imported = Path(td) / 'Imported-Report.json'
    imported.write_text('\ufeff' + Path(a.report).read_text(), encoding='utf-8')
    app.report_path.set(str(imported)); app.acpi_path.set(str(Path(a.report).parent / 'ACPI'))
    def wait():
        deadline = time.monotonic() + 240
        while app.busy and time.monotonic() < deadline:
            app.update(); time.sleep(.02)
        assert not app.busy, 'GUI child operation timed out'
        # Flush completion callbacks before observing the state.
        app.update()
    app.check(); wait()
    assert app.analysis_current() and app.analysis['ok'], app.log.get('1.0', 'end')
    # Check twice: a failed/stale prior output must never win.
    app.check(); wait()
    assert app.analysis_current() and app.analysis['ok'], app.log.get('1.0', 'end')
    app.prepare(); wait()
    assert app.build_ready(), app.log.get('1.0', 'end')
    assert (app.work / 'BIOS.txt').is_file(), app.log.get('1.0', 'end')
    assert '5. Установите macOS' in app.guide_text.get('1.0', 'end')
    assert app.current_page == 'guide'
    assert not app.wizard
    # A fresh run can reuse a valid build and regenerate its guide without scanning or erasing.
    app.prepare(); wait()
    assert app.current_page == 'guide' and app.build_ready()
    # A malformed import is displayed and disables build, then a correct import recovers.
    bad = Path(td) / 'bad.json'; bad.write_text('broken')
    app.report_path.set(str(bad)); app.check(); wait()
    assert app.analysis and not app.analysis['ok']
    assert app.build_button.cget('state') == 'disabled'
    app.report_path.set(a.report); app.check(); wait()
    assert app.analysis_current() and app.analysis['ok']
    app.destroy()
print('Real GUI subprocesses, repeated JSON checks, automatic EFI/guide, reuse and error recovery passed.')
