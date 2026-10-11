"""Native Linux/Xvfb GUI checks and screenshots, optionally with real corpus results."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile
from unittest.mock import patch

ap = argparse.ArgumentParser()
ap.add_argument('--output', required=True); ap.add_argument('--report'); ap.add_argument('--analysis')
a = ap.parse_args()
output = Path(a.output).resolve(); output.mkdir(parents=True, exist_ok=True)
with tempfile.TemporaryDirectory() as td:
    os.environ['XDG_CONFIG_HOME'] = td
    from p1401.linux_gui import Application
    from p1401.linux_hardware import SCHEMA
    app = Application()
    app.workspace.set(td)
    app.update_idletasks()
    def capture(name):
        app.update()
        region = f'{app.winfo_rootx()},{app.winfo_rooty()},{app.winfo_width()},{app.winfo_height()}'
        subprocess.run(['scrot', '-a', region, str(output / name)], check=True)
    capture('overview.png')
    assert app.build_button.cget('state') == 'disabled'
    # Every scan gets a fresh directory, including after a previous failure.
    with patch.object(app, 'start') as start:
        app.scan(); first = app.report_file
        app.scan(); second = app.report_file
        assert first != second and start.call_count == 2
        assert Path(first).parent.is_dir() and Path(second).parent.is_dir()
    app.report_path.set(str(Path(td) / 'missing.json'))
    # Failed checks must never reload a previous compatibility snapshot.
    with patch.object(app, 'start') as start:
        app.check(); args = start.call_args.args[0]; first_output = args[args.index('--output') + 1]
        app.check(); args = start.call_args.args[0]; second_output = args[args.index('--output') + 1]
        assert first_output != second_output
    app.wizard = True
    with patch.object(app, 'start') as start:
        app.prepare()
        assert start.call_args.args[0][0] == 'scan'
    if a.report:
        app.report_path.set(a.report)
        app.acpi_path.set(str(Path(a.report).parent / 'ACPI'))
        app.wizard = False
        app.load_analysis(a.analysis)
        app.show('hardware'); app.update()
        assert len(app.hardware_rows.winfo_children()) > 4
        assert app.analysis_current()
        assert app.build_button.cget('state') == 'normal', app.analysis
        capture('hardware.png')
        # Wizard reuses a current report, checks it, then advances only with ACPI.
        with patch.object(app, 'check') as check, patch.object(app, 'build_ready', return_value=False):
            app.prepare()
            check.assert_called_once()
            assert app.wizard
        with patch.object(app, 'build') as build:
            app.continue_prepare()
            build.assert_called_once()
        with patch.object(app, 'acpi_ready', return_value=False), patch.object(app, 'build') as build:
            app.continue_prepare()
            build.assert_not_called()
            assert not app.wizard
        # Old checker snapshots must be rejected even for an unchanged report.
        app.analysis['schema'] = 1
        assert not app.analysis_current()
        app.analysis['schema'] = SCHEMA
        # A changed target invalidates the old analysis and disables build immediately.
        app.macos.set('macOS Tahoe 26'); app.update_gates()
        assert app.build_button.cget('state') == 'disabled'
    app.destroy()
print('CustomTkinter window, device rendering and build gating passed.')
