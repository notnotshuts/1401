"""Run the installed scanner on Linux; hosted VMs need not expose ACPI or a GPU."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import pwd

with tempfile.TemporaryDirectory(prefix='1401-live-scan-') as td:
    parent = Path(td)
    owner = int(os.environ.get('SUDO_UID', os.getuid()))
    os.chown(parent, owner, pwd.getpwuid(owner).pw_gid)
    for index in range(2):
        path = parent / f'run-{index}'
        result = subprocess.run(['/usr/bin/1401', 'scan', str(path)], capture_output=True, text=True,
                                encoding='utf-8', errors='replace', timeout=180)
        assert result.returncode in (0, 1), result.stdout + result.stderr
        assert (path / 'Report.json').is_file(), result.stdout + result.stderr
        report = json.loads((path / 'Report.json').read_text())
        assert report.get('CPU'), result.stdout + result.stderr
        assert int(report['CPU']['Core Count']) <= int(report['CPU']['Thread Count'])
        assert report.get('Motherboard'), result.stdout + result.stderr
        assert 'Network' in report and 'USB Controllers' in report and 'Input' in report
        receipt = path / 'scan-evidence.json'
        assert json.loads(receipt.read_text())['phase'] == 'complete'
        subprocess.run(['sudo', '-u', '#' + str(owner), '/usr/bin/python3', '-c', 'import pathlib,sys; pathlib.Path(sys.argv[1]).read_text()', str(receipt)], check=True)
        assert receipt.stat().st_uid == owner, 'Root-created evidence is unreadable by its desktop owner'
        assert (path / 'Report.json').stat().st_uid == owner
        assert 'Traceback' not in result.stderr, result.stderr
        print(f'Native scan {index + 1}: rc={result.returncode}, CPU={report["CPU"]["Core Count"]} cores, report and evidence saved.')
print('Installed native scan, fresh rerun and root-to-desktop file ownership passed.')
