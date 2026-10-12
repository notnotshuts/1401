#!/usr/bin/env python3
"""Development-only: export pinned, auditable tables. Ferrite runtime never executes Python."""
import json
import pathlib
import runpy

ROOT = pathlib.Path(__file__).resolve().parents[2]
DEST = ROOT / 'crates/ferrite-core/data'
pci = runpy.run_path(str(ROOT / 'upstream/OpCore-Simplify/Scripts/datasets/pci_data.py'))
codec = runpy.run_path(str(ROOT / 'upstream/OpCore-Simplify/Scripts/datasets/codec_layouts.py'))
data = {k: v for k, v in pci.items() if not k.startswith('_') and isinstance(v, (list, dict))}
data['AudioCodecIDs'] = list(codec['data'])
(DEST / 'pci-rules.json').write_text(json.dumps(data, sort_keys=True, indent=2) + '\n')
identifier = runpy.run_path(str(ROOT / 'upstream/Hardware-Sniffer/Scripts/gpu_identifier.py'))['GPUIdentifier']()
ranges = {}
for vendor in ['8086', '1002', '10DE']:
    rows = []
    for pid in range(65536):
        result = identifier.classify_gpu(f'{vendor}-{pid:04X}')
        code, kind = result['Codename'], result['Device Type']
        if rows and rows[-1]['codename'] == code and rows[-1]['device_type'] == kind:
            rows[-1]['end'] = pid
        else:
            rows.append({'start': pid, 'end': pid, 'codename': code, 'device_type': kind})
    ranges[vendor] = rows
(DEST / 'gpu-ranges.json').write_text(json.dumps(ranges, indent=2) + '\n')
(DEST / 'nvidia-gsp.json').write_bytes((ROOT / 'p1401/nvidia_gsp_ids.json').read_bytes())
