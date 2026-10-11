"""Native Linux/Xvfb GUI checks and screenshots, optionally with real corpus results."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile

ap = argparse.ArgumentParser()
ap.add_argument('--output', required=True); ap.add_argument('--report'); ap.add_argument('--analysis')
a = ap.parse_args()
output = Path(a.output).resolve(); output.mkdir(parents=True, exist_ok=True)
with tempfile.TemporaryDirectory() as td:
    os.environ['XDG_CONFIG_HOME'] = td
    from p1401.linux_gui import Application
    app = Application()
    app.workspace.set(td)
    app.update_idletasks()
    def capture(name):
        app.update()
        region = f'{app.winfo_rootx()},{app.winfo_rooty()},{app.winfo_width()},{app.winfo_height()}'
        subprocess.run(['scrot', '-a', region, str(output / name)], check=True)
    capture('overview.png')
    assert app.build_button.cget('state') == 'disabled'
    if a.report:
        app.report_path.set(a.report)
        app.acpi_path.set(str(Path(a.report).parent / 'ACPI'))
        app.load_analysis(a.analysis)
        app.show('hardware'); app.update()
        assert len(app.hardware_rows.winfo_children()) > 4
        assert app.analysis_current()
        assert app.build_button.cget('state') == 'normal', app.analysis
        capture('hardware.png')
        # A changed target invalidates the old analysis and disables build immediately.
        app.macos.set('macOS Tahoe 26'); app.update_gates()
        assert app.build_button.cget('state') == 'disabled'
    app.destroy()
print('CustomTkinter window, device rendering and build gating passed.')
