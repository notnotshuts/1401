"""Linux fixes around the pinned Hardware Sniffer; raw observations stay explicit."""
from pathlib import Path
import os

STEPS = [('PCI', 'pci_devices', None), ('Motherboard', 'motherboard', 'Motherboard'),
         ('BIOS', 'bios', 'BIOS'), ('CPU', 'cpu', 'CPU'), ('GPU', 'gpu', 'GPU'),
         ('Monitor', 'monitor', 'Monitor'), ('Network', 'network', 'Network'),
         ('Sound', 'sound', 'Sound'), ('USB Controllers', 'usb_controllers', 'USB Controllers'),
         ('Input', 'input', 'Input'), ('Storage Controllers', 'storage_controllers', 'Storage Controllers'),
         ('Biometric', 'biometric', 'Biometric'), ('Bluetooth', 'bluetooth', 'Bluetooth'),
         ('SD Controller', 'sd_controller', 'SD Controller'), ('System Devices', 'system_devices', 'System Devices')]


def text(path):
    try: return Path(path).read_text(encoding='utf-8', errors='replace').strip()
    except OSError: return ''


def cpu(info, root=Path('/')):
    blocks = []
    for block in text(root / 'proc/cpuinfo').split('\n\n'):
        row = dict(line.split(':', 1) for line in block.splitlines() if ':' in line)
        row = {k.strip(): v.strip() for k, v in row.items()}
        if 'processor' in row: blocks.append(row)
    if not blocks: raise ValueError('Не удалось прочитать /proc/cpuinfo.')
    first = blocks[0]
    vendor = {'GenuineIntel': 'Intel', 'AuthenticAMD': 'AMD'}.get(first.get('vendor_id'))
    if not vendor: raise ValueError('Ожидается процессор Intel или AMD x86_64.')
    cores, packages = set(), set()
    for row in blocks:
        topology = root / 'sys/devices/system/cpu' / ('cpu' + row['processor']) / 'topology'
        package = row.get('physical id') or text(topology / 'physical_package_id')
        core = row.get('core id') or text(topology / 'core_id')
        if package and core:
            cores.add((package, core)); packages.add(package)
        else:
            raise ValueError('Не удалось определить физические ядра CPU: отсутствует topology/core_id.')
    flags = set.intersection(*(set(row.get('flags', '').split()) for row in blocks))
    aliases = [('SSE', 'sse'), ('SSE2', 'sse2'), ('SSE3', 'pni'), ('SSSE3', 'ssse3'),
               ('SSE4.1', 'sse4_1'), ('SSE4.2', 'sse4_2'), ('SSE4a', 'sse4a'), ('AVX', 'avx'), ('AVX2', 'avx2')]
    features = ', '.join(name for name, flag in aliases if flag in flags or (name == 'SSE3' and 'sse3' in flags))
    name = first.get('model name', '').replace(' CPU', '').split('@')[0].strip()
    description = f"Family {first.get('cpu family')} Model {first.get('model')} Stepping {first.get('stepping')}"
    return {'Manufacturer': vendor, 'Processor Name': name or 'Unknown',
            'Codename': info.lookup_codename(name, description) or 'Unknown',
            'Core Count': str(len(cores)), 'CPU Count': str(len(packages)),
            'Thread Count': str(len(blocks)), 'SIMD Features': features or 'Unknown'}


def motherboard(info, root=Path('/')):
    dmi = root / 'sys/class/dmi/id'
    def usable(value):
        return value if value and not any(part in value.lower() for part in ('o.e.m.', 'unknown', 'manufacturer', 'default string', 'product name')) else ''
    vendor = usable(text(dmi / 'board_vendor')) or usable(text(dmi / 'sys_vendor'))
    model = usable(text(dmi / 'board_name')) or usable(text(dmi / 'product_name'))
    name = ' '.join(p for p in (vendor, model) if p).upper() or 'Unknown'
    chassis = text(dmi / 'chassis_type')
    platform = 'Laptop' if chassis in {'8', '9', '10', '11', '14', '30', '31', '32'} else 'Desktop' if chassis.isdigit() and chassis not in {'0', '1', '2'} else 'Unspecified'
    chipset = 'Unknown'
    from Scripts.datasets import chipset_data
    for device in info.devices_by_class.get('ISA bridge', []):
        chipset = chipset_data.chipset_controllers.get(device.get('Device ID'), chipset)
    for candidate in chipset_data.amd_chipsets:
        if candidate in name: chipset = candidate; break
    return {'Name': name, 'Chipset': chipset, 'Platform': platform}


def monitor(info):
    monitors = {}
    for name, gpu in info.result.get('GPU', {}).items():
        device = gpu.pop('Device Path', None)
        if not device or not Path(device).is_dir(): continue
        for relative, _ in info.utils.find_matching_paths(device, name_filter='edid', type_filter='file'):
            connector = Path(device) / relative
            parent = connector.parent
            if text(parent / 'status') != 'connected': continue
            modes = [mode for mode in text(parent / 'modes').splitlines() if 'x' in mode and all(n.isdigit() for n in mode.split('x'))]
            if not modes: continue
            mode = max(modes, key=lambda s: tuple(int(n) for n in s.split('x')))
            kind = parent.name.split('-', 1)[-1].rsplit('-', 1)[0]
            kind = 'HDMI' if kind.startswith('HDMI') else 'DVI' if kind.startswith('DVI') else kind
            if kind not in {'VGA', 'DVI', 'HDMI', 'LVDS', 'DP', 'eDP', 'Internal'}: kind = 'Uninitialized'
            label = parent.name
            try:
                edid = connector.read_bytes()
                parsed = info.parse_edid(edid) if len(edid) >= 128 else {}
                if parsed.get('manufacturer_id') and parsed.get('product_code'):
                    label = parsed['manufacturer_id'] + parsed['product_code']
            except (OSError, ValueError, TypeError): pass
            monitors[info.utils.get_unique_key(label, monitors)] = {'Connector Type': kind, 'Resolution': mode, 'Connected GPU': name}
    return monitors


def gpu_fallback(info, found, pci_root=Path('/sys/bus/pci/devices')):
    """A missing DRM driver must not hide a PCI graphics adapter from the report."""
    existing_devices = {os.path.realpath(g['Device Path']) for g in found.values() if g.get('Device Path')}
    existing = {g.get('PCI Path') for g in found.values() if g.get('PCI Path')}
    for path in sorted(pci_root.glob('*')):
        if os.path.realpath(path) in existing_devices or not text(path / 'class').lower().startswith('0x03'): continue
        location = info.get_device_location_paths(str(path))
        if location.get('PCI Path') in existing: continue
        vendor, device = text(path / 'vendor'), text(path / 'device')
        if not vendor.startswith('0x') or not device.startswith('0x'): continue
        identifier = f'{vendor[2:]:0>4}-{device[2:]:0>4}'.upper()
        classification = info.classify_gpu(identifier)
        name, _ = info.get_pci_device_name_and_class(path.name)
        if name == 'Unknown': name = 'PCI GPU ' + identifier
        props = {'Manufacturer': classification.get('Manufacturer', 'Unknown'),
                 'Codename': classification.get('Codename', 'Unknown'),
                 'Device Type': classification.get('Device Type', 'Unknown'),
                 'Device ID': identifier, 'Bus Type': 'PCI', 'Device Path': str(path), **location}
        subsys, subvendor = text(path / 'subsystem_device'), text(path / 'subsystem_vendor')
        if subsys.startswith('0x') and subvendor.startswith('0x'): props['Subsystem ID'] = (subsys[2:] + subvendor[2:]).upper()
        # PCI paths usually deduplicate DRM and sysfs; matching an identity without a path cannot establish that two adapters are identical.
        found[info.utils.get_unique_key(name, found)] = props
    return found


def install(info, checkpoint=lambda: None):
    original_gpu, original_storage, original_bios = info.gpu, info.storage_controllers, info.bios
    info.cpu = lambda: cpu(info)
    info.motherboard = lambda: motherboard(info)
    info.monitor = lambda: monitor(info)
    info.gpu = lambda: gpu_fallback(info, original_gpu())
    def storage():
        # Linux classifies VMD / RAID separately; include it so it cannot pass as ordinary NVMe.
        controllers = info.devices_by_class
        combined = [d for group in ('Non-Volatile memory controller', 'RAID bus controller', 'Mass storage controller') for d in controllers.get(group, [])]
        controllers['Non-Volatile memory controller'] = combined
        result = original_storage()
        # Empty model files must not produce a JSON null in Disk Drives.
        for props in result.values():
            if 'Disk Drives' in props: props['Disk Drives'] = [name for name in props['Disk Drives'] if isinstance(name, str) and name.strip()]
        return result
    info.storage_controllers = storage
    def bios():
        result = original_bios()
        path = Path('/sys/firmware/efi/efivars/SecureBoot-8be4df61-93ca-11d2-aa0d-00e098032b8c')
        try:
            value = path.read_bytes()
            if len(value) == 5 and value[4] in (0, 1): result['Secure Boot'] = 'Enabled' if value[4] else 'Disabled'
            else: raise ValueError('Неверный формат SecureBoot efivar.')
        except (OSError, ValueError):
            # An unreadable efivar is unknown, never evidence that Secure Boot is off.
            result['Secure Boot'] = 'Unknown'
            info.notes.append('Secure Boot: состояние не прочитано; проверьте его в BIOS перед загрузкой USB.')
        return result
    info.bios = bios
    def collect():
        info.result = {}; info.errors = []; info.notes = []
        info.utils.head = lambda *a, **k: None
        for index, (section, method, key) in enumerate(STEPS, 1):
            print(f'[{index}/{len(STEPS)}] {section}…', flush=True)
            try:
                value = getattr(info, method)()
                if key: info.result[key] = value or {}
            except Exception as error:
                info.errors.append(f'{section}: {type(error).__name__}: {error}')
                print(info.errors[-1], flush=True)
                if key: info.result[key] = {}
            checkpoint()
        if info.errors: info.result['Scan Errors'] = info.errors
        if info.notes: info.result['Scan Notes'] = info.notes
    info.hardware_collector = collect
