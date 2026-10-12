#![cfg(target_os = "windows")]
use crate::{cpu, planner, report::Report};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, mem::size_of, path::Path, ptr};
use windows_sys::Win32::{
    Devices::DeviceAndDriverInstallation::*, Foundation::ERROR_NO_MORE_ITEMS,
    System::SystemInformation::*,
};
fn first(v: &[u16]) -> String {
    String::from_utf16_lossy(&v[..v.iter().position(|c| *c == 0).unwrap_or(v.len())])
}
struct Set(HDEVINFO);
impl Drop for Set {
    fn drop(&mut self) {
        unsafe {
            SetupDiDestroyDeviceInfoList(self.0);
        }
    }
}
fn property(set: HDEVINFO, dev: &SP_DEVINFO_DATA, key: u32) -> String {
    let mut buffer = vec![0u16; 32768];
    let mut kind = 0;
    let mut required = 0;
    // SAFETY: SetupAPI owns set; SP_DEVINFO_DATA and appropriately sized buffer are live for this call.
    let ok = unsafe {
        SetupDiGetDeviceRegistryPropertyW(
            set,
            dev,
            key,
            &mut kind,
            buffer.as_mut_ptr().cast(),
            (buffer.len() * 2) as u32,
            &mut required,
        )
    };
    if ok == 0 || required as usize > buffer.len() * 2 {
        return String::new();
    }
    first(&buffer)
}
fn instance(set: HDEVINFO, dev: &SP_DEVINFO_DATA) -> Result<String> {
    let mut buffer = vec![0u16; 4096];
    let mut needed = 0;
    if unsafe {
        SetupDiGetDeviceInstanceIdW(
            set,
            dev,
            buffer.as_mut_ptr(),
            buffer.len() as u32,
            &mut needed,
        )
    } == 0
    {
        bail!("Cannot query PnP instance");
    }
    Ok(first(&buffer))
}
fn firmware(provider: u32, id: u32) -> Result<Vec<u8>> {
    let size = unsafe { GetSystemFirmwareTable(provider, id, ptr::null_mut(), 0) };
    if size == 0 || size > 16 * 1024 * 1024 {
        bail!("Firmware table unavailable/oversized");
    }
    let mut bytes = vec![0u8; size as usize];
    let got = unsafe { GetSystemFirmwareTable(provider, id, bytes.as_mut_ptr(), size) };
    if got != size {
        bail!("Firmware table changed");
    }
    Ok(bytes)
}
fn smbios() -> Result<(Value, Value, Option<u64>)> {
    let raw = firmware(u32::from_be_bytes(*b"RSMB"), 0)?;
    if raw.len() < 8 {
        bail!("SMBIOS header");
    }
    let length = u32::from_le_bytes(raw[4..8].try_into()?) as usize;
    if length != raw.len() - 8 {
        bail!("SMBIOS length");
    }
    let mut board = json!({"Platform":"Unknown"});
    let mut bios = json!({});
    let mut core_count = None;
    let mut offset = 8;
    while offset + 4 <= raw.len() {
        let kind = raw[offset];
        let len = raw[offset + 1] as usize;
        if len < 4 || offset + len > raw.len() {
            bail!("Invalid SMBIOS structure");
        }
        let data = &raw[offset..offset + len];
        let start = offset + len;
        let mut end = start;
        while end + 1 < raw.len() && !(raw[end] == 0 && raw[end + 1] == 0) {
            end += 1;
        }
        if end + 1 >= raw.len() {
            bail!("SMBIOS string terminator");
        }
        let strings: Vec<_> = raw[start..end]
            .split(|b| *b == 0)
            .map(|s| String::from_utf8_lossy(s).to_string())
            .collect();
        let string = |index: usize| -> String {
            let n = data.get(index).copied().unwrap_or(0);
            if n == 0 {
                String::new()
            } else {
                strings.get(n as usize - 1).cloned().unwrap_or_default()
            }
        };
        match kind {
            0 => {
                bios["Vendor"] = json!(string(4));
                bios["Version"] = json!(string(5));
            }
            2 => {
                board["Manufacturer"] = json!(string(4));
                board["Name"] = json!(string(5));
            }
            3 => {
                if let Some(chassis) = data.get(5) {
                    board["Platform"] =
                        json!(if [8, 9, 10, 14, 30, 31, 32].contains(&(chassis & 0x7f)) {
                            "Laptop"
                        } else {
                            "Desktop"
                        });
                }
            }
            4 => {
                if let Some(count) = data.get(35) {
                    if *count != 0 && *count != 255 {
                        core_count = Some(*count as u64);
                    } else if data.len() >= 44 {
                        core_count = Some(u16::from_le_bytes(data[42..44].try_into()?) as u64);
                    }
                }
            }
            127 => break,
            _ => {}
        }
        offset = end + 2;
    }
    let mut firmware_type = 0;
    if unsafe { GetFirmwareType(&mut firmware_type) } != 0 {
        bios["Firmware Type"] = json!(if firmware_type == FirmwareTypeUefi {
            "UEFI"
        } else {
            "Legacy"
        });
    }
    Ok((board, bios, core_count))
}
#[cfg(target_arch = "x86_64")]
fn processor() -> Value {
    use std::arch::x86_64::__cpuid;
    // SAFETY: CPUID is available on the supported x86_64 target. Extended leaves are bounded by maximum leaf.
    let root = __cpuid(0);
    let mut vendor = vec![];
    for v in [root.ebx, root.edx, root.ecx] {
        vendor.extend(v.to_le_bytes());
    }
    let manufacturer = match String::from_utf8_lossy(&vendor).as_ref() {
        "GenuineIntel" => "Intel",
        "AuthenticAMD" => "AMD",
        _ => "Unknown",
    };
    let leaf = __cpuid(1);
    let base_family = (leaf.eax >> 8) & 15;
    let family = if base_family == 15 {
        base_family + ((leaf.eax >> 20) & 255)
    } else {
        base_family
    };
    let base_model = (leaf.eax >> 4) & 15;
    let model = if matches!(base_family, 6 | 15) {
        base_model + (((leaf.eax >> 16) & 15) << 4)
    } else {
        base_model
    };
    let stepping = leaf.eax & 15;
    let max = __cpuid(0x80000000).eax;
    let mut brand = vec![];
    if max >= 0x80000004 {
        for n in 0x80000002..=0x80000004 {
            let v = __cpuid(n);
            for n in [v.eax, v.ebx, v.ecx, v.edx] {
                brand.extend(n.to_le_bytes());
            }
        }
    }
    let name = String::from_utf8_lossy(&brand)
        .trim_matches(|c: char| c == '\0' || c.is_whitespace())
        .to_string();
    let mut simd = vec![];
    if leaf.ecx & (1 << 19) != 0 {
        simd.push("SSE4.1");
    }
    if leaf.ecx & (1 << 20) != 0 {
        simd.push("SSE4.2");
    }
    if root.eax >= 7 && std::arch::x86_64::__cpuid_count(7, 0).ebx & (1 << 5) != 0 {
        simd.push("AVX2");
    }
    let identifier = format!("Family {family} Model {model} Stepping {stepping}");
    json!({"Processor Name":name,"Manufacturer":manufacturer,"Codename":cpu::codename(&name,&identifier),"SIMD Features":simd.join(" "),"CPU Count":1})
}
#[cfg(not(target_arch = "x86_64"))]
fn processor() -> Value {
    json!({"Processor Name":"Unsupported architecture","SIMD Features":"Unknown"})
}
fn pci_path(location: &str) -> Option<String> {
    let root = regex::Regex::new(r"^PCIROOT\(([0-9A-Fa-f]+)\)")
        .ok()?
        .captures(location)?;
    let mut result = format!("PciRoot(0x{:X})", u64::from_str_radix(&root[1], 16).ok()?);
    let re = regex::Regex::new(r"#PCI\(([0-9A-Fa-f]{2})([0-9A-Fa-f]{2})\)").ok()?;
    for c in re.captures_iter(location) {
        result.push_str(&format!(
            "/Pci(0x{:X},0x{:X})",
            u8::from_str_radix(&c[1], 16).ok()?,
            u8::from_str_radix(&c[2], 16).ok()?
        ));
    }
    Some(result)
}
pub fn scan() -> Result<Report> {
    let (board, bios, cores) = smbios()?;
    let mut cpu = processor();
    if let Some(cores) = cores {
        cpu["Core Count"] = json!(cores);
    }
    let mut hardware = json!({"CPU":cpu,"Motherboard":board,"BIOS":bios,"GPU":{},"Network":{},"Sound":{},"Storage Controllers":{},"Bluetooth":{},"SD Controller":{},"USB Controllers":{},"System Devices":{},"Input":{},"Monitor":{},"Scan Notes":["Windows scan использует SetupAPI, CPUID и SMBIOS. Разводка экрана/MUX и battery patches требуют проверки на самом ПК."]});
    let handle = unsafe {
        SetupDiGetClassDevsW(
            ptr::null(),
            ptr::null(),
            ptr::null_mut(),
            DIGCF_ALLCLASSES | DIGCF_PRESENT,
        )
    };
    if handle == -1 {
        bail!("SetupAPI unavailable");
    }
    let _guard = Set(handle);
    let pci = regex::Regex::new(r"PCI\\VEN_([0-9A-Fa-f]{4})&DEV_([0-9A-Fa-f]{4})")?;
    let usb = regex::Regex::new(r"USB\\VID_([0-9A-Fa-f]{4})&PID_([0-9A-Fa-f]{4})")?;
    let hda =
        regex::Regex::new(r"HDAUDIO\\FUNC_[0-9A-Fa-f]+&VEN_([0-9A-Fa-f]{4})&DEV_([0-9A-Fa-f]{4})")?;
    let subsystem = regex::Regex::new(r"SUBSYS_([0-9A-Fa-f]{8})")?;
    let mut controllers = BTreeMap::new();
    let mut disk_devices = vec![];
    for i in 0..16384 {
        let mut dev: SP_DEVINFO_DATA = unsafe { std::mem::zeroed() };
        dev.cbSize = size_of::<SP_DEVINFO_DATA>() as u32;
        if unsafe { SetupDiEnumDeviceInfo(handle, i, &mut dev) } == 0 {
            if std::io::Error::last_os_error().raw_os_error() == Some(ERROR_NO_MORE_ITEMS as i32) {
                break;
            }
            return Err(std::io::Error::last_os_error().into());
        }
        let id = property(handle, &dev, SPDRP_HARDWAREID).to_uppercase();
        let class = property(handle, &dev, SPDRP_CLASS);
        let friendly = property(handle, &dev, SPDRP_FRIENDLYNAME);
        let name = if friendly.is_empty() {
            property(handle, &dev, SPDRP_DEVICEDESC)
        } else {
            friendly
        };
        let instance = instance(handle, &dev)?;
        if class == "DiskDrive" {
            disk_devices.push((dev.DevInst, name));
            continue;
        }
        let (bus, caps) = if let Some(c) = pci.captures(&id) {
            ("PCI", Some((c[1].to_string(), c[2].to_string())))
        } else if let Some(c) = usb.captures(&id) {
            ("USB", Some((c[1].to_string(), c[2].to_string())))
        } else if let Some(c) = hda.captures(&id) {
            ("HDA", Some((c[1].to_string(), c[2].to_string())))
        } else {
            ("ACPI", None)
        };
        let mut props = json!({"Bus Type":if bus=="HDA"{"PCI"}else{bus},"Instance ID":instance});
        let identifier = caps.map(|(v, d)| format!("{v}-{d}")).unwrap_or_default();
        props["Device ID"] = json!(identifier);
        let location = property(handle, &dev, SPDRP_LOCATION_PATHS);
        if let Some(path) = pci_path(&location) {
            props["PCI Path"] = json!(path);
        }
        if let Some(s) = subsystem.captures(&id) {
            props["Subsystem ID"] = json!(s[1].to_string());
        }
        let section = if bus == "HDA" {
            "Sound"
        } else if class == "Display" {
            "GPU"
        } else if class == "Net" {
            "Network"
        } else if matches!(class.as_str(), "SCSIAdapter" | "HDC") {
            "Storage Controllers"
        } else if class == "Bluetooth" || planner::table("BluetoothIDs", &identifier) {
            "Bluetooth"
        } else if class == "MEDIA" {
            "Sound"
        } else if class == "USB" {
            "USB Controllers"
        } else {
            "System Devices"
        };
        if section == "GPU" {
            let (manufacturer, code, kind) = crate::compatibility::classify_gpu(&identifier);
            props["Manufacturer"] = json!(manufacturer);
            props["Codename"] = json!(code);
            props["Device Type"] = json!(kind);
        }
        let label = format!("{name} · {instance}");
        if section == "Storage Controllers" {
            props["Disk Drives"] = json!([]);
            controllers.insert(dev.DevInst, label.clone());
        }
        hardware[section]
            .as_object_mut()
            .context("Hardware section")?
            .insert(label, props);
    }
    for (device, name) in disk_devices {
        let mut current = device;
        for _ in 0..32 {
            if let Some(label) = controllers.get(&current) {
                hardware["Storage Controllers"][label]["Disk Drives"]
                    .as_array_mut()
                    .context("Disk association")?
                    .push(json!(name));
                break;
            }
            let mut parent = 0;
            if unsafe { CM_Get_Parent(&mut parent, current, 0) } != CR_SUCCESS {
                break;
            }
            current = parent;
        }
    }
    Report::from_bytes(&serde_json::to_vec(&hardware)?)
}
pub fn capture_acpi(destination: &Path) -> Result<usize> {
    fs::create_dir_all(destination)?;
    let provider = u32::from_be_bytes(*b"ACPI");
    let size = unsafe { EnumSystemFirmwareTables(provider, ptr::null_mut(), 0) };
    if size == 0 || size > 65536 || size % 4 != 0 {
        bail!("ACPI enumeration unavailable");
    }
    let mut ids = vec![0u8; size as usize];
    if unsafe { EnumSystemFirmwareTables(provider, ids.as_mut_ptr(), size) } != size {
        bail!("ACPI table list changed");
    }
    let mut dsdt = false;
    let mut count = 0;
    for id in ids.as_chunks::<4>().0 {
        let bytes = firmware(provider, u32::from_le_bytes(*id))?;
        let table = crate::acpi::validate_table(&bytes, "firmware")?;
        if table.signature == "DSDT" {
            dsdt = true;
        }
        let file = destination.join(format!("{}-{count}.aml", table.signature));
        fs::write(file, bytes)?;
        count += 1;
    }
    if !dsdt {
        bail!(
            "Windows не предоставил DSDT через firmware API. Импортируйте ACPI из Hardware Sniffer или acpidump; частичный дамп не используется для EFI."
        );
    }
    Ok(count)
}
