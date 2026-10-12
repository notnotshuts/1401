//! Read-only macOS adapters. Parsers are host independent and tested on every platform.
use crate::{compatibility, disks::Disk, report::Report, workspace};
use anyhow::{Context, Result, bail};
use plist::Value as Plist;
use serde_json::{Value, json};
use std::{collections::BTreeMap, io::Cursor, path::Path, time::Duration};
fn command(program: &str, args: &[&str]) -> Result<Vec<u8>> {
    workspace::run_bounded(program, args, Duration::from_secs(60))
}
fn sysctl(key: &str) -> String {
    command("/usr/sbin/sysctl", &["-n", key])
        .ok()
        .map(|r| String::from_utf8_lossy(&r).trim().to_string())
        .unwrap_or_default()
}
pub fn scan() -> Result<Report> {
    let raw = command(
        "/usr/sbin/system_profiler",
        &[
            "-json",
            "-detailLevel",
            "mini",
            "SPHardwareDataType",
            "SPDisplaysDataType",
        ],
    )?;
    let profiler: Value = serde_json::from_slice(&raw).context("system_profiler JSON")?;
    let pci = command("/usr/sbin/ioreg", &["-a", "-r", "-c", "IOPCIDevice"])
        .ok()
        .and_then(|raw| Plist::from_reader(Cursor::new(raw)).ok());
    let cpu_keys = [
        "machdep.cpu.brand_string",
        "machdep.cpu.vendor",
        "machdep.cpu.family",
        "machdep.cpu.model",
        "machdep.cpu.stepping",
        "machdep.cpu.features",
        "machdep.cpu.leaf7_features",
        "hw.physicalcpu",
        "hw.optional.arm64",
    ];
    let cpu = cpu_keys
        .into_iter()
        .map(|key| (key.to_string(), sysctl(key)))
        .collect();
    parse_report(&profiler, pci.as_ref(), &cpu)
}
fn value<'a>(json: &'a Value, key: &str) -> &'a str {
    json[key].as_str().unwrap_or("")
}
fn plist_u32(value: &Plist) -> Option<u32> {
    if let Some(data) = value.as_data() {
        let bytes: [u8; 4] = data.get(..4)?.try_into().ok()?;
        return Some(u32::from_le_bytes(bytes));
    }
    value
        .as_unsigned_integer()
        .and_then(|n| u32::try_from(n).ok())
}
fn plist_text(value: &Plist) -> String {
    if let Some(s) = value.as_string() {
        return s.into();
    }
    value
        .as_data()
        .map(|d| {
            String::from_utf8_lossy(d)
                .trim_end_matches('\0')
                .to_string()
        })
        .unwrap_or_default()
}
fn visit(value: &Plist, depth: usize, job: &mut impl FnMut(&plist::Dictionary)) -> Result<()> {
    if depth > 64 {
        bail!("IORegistry nesting exceeds limit");
    }
    match value {
        Plist::Array(items) => {
            for item in items {
                visit(item, depth + 1, job)?;
            }
        }
        Plist::Dictionary(d) => {
            job(d);
            if let Some(children) = d.get("IORegistryEntryChildren") {
                visit(children, depth + 1, job)?;
            }
        }
        _ => {}
    }
    Ok(())
}
pub fn parse_report(
    profiler: &Value,
    pci: Option<&Plist>,
    cpu: &BTreeMap<String, String>,
) -> Result<Report> {
    let hardware = profiler["SPHardwareDataType"]
        .as_array()
        .and_then(|a| a.first())
        .context("Hardware profile is missing")?;
    let get = |key: &str| cpu.get(key).map(String::as_str).unwrap_or("");
    let arm = get("hw.optional.arm64") == "1" || value(hardware, "chip_type").starts_with("Apple");
    let name = if arm {
        value(hardware, "chip_type")
    } else {
        get("machdep.cpu.brand_string")
    };
    let manufacturer = if arm {
        "Apple"
    } else {
        match get("machdep.cpu.vendor") {
            "GenuineIntel" => "Intel",
            "AuthenticAMD" => "AMD",
            _ => "Unknown",
        }
    };
    let identifier = format!(
        "Family {} Model {} Stepping {}",
        get("machdep.cpu.family"),
        get("machdep.cpu.model"),
        get("machdep.cpu.stepping")
    );
    let code = if arm {
        "Apple Silicon".to_string()
    } else {
        crate::cpu::codename(name, &identifier)
    };
    let flags = format!(
        "{} {}",
        get("machdep.cpu.features"),
        get("machdep.cpu.leaf7_features")
    );
    let model = value(hardware, "machine_model");
    let mut result = json!({"CPU":{"Processor Name":name,"Manufacturer":manufacturer,"Codename":code,"Core Count":get("hw.physicalcpu"),"CPU Count":hardware["number_processors"].clone(),"SIMD Features":flags},"Motherboard":{"Name":model,"Manufacturer":"Apple","Platform":if model.starts_with("MacBook"){"Laptop"}else{"Desktop"}},"BIOS":{"Vendor":"Apple","Version":value(hardware,"boot_rom_version"),"Firmware Type":if arm{"Apple Silicon"}else{"UEFI"}},"GPU":{},"Network":{},"Storage Controllers":{},"Sound":{},"USB Controllers":{},"System Devices":{},"Monitor":{}});
    let mut count = 0;
    if let Some(pci) = pci {
        visit(pci, 0, &mut |d| {
            let Some(vendor) = d.get("vendor-id").and_then(plist_u32) else {
                return;
            };
            let Some(device) = d.get("device-id").and_then(plist_u32) else {
                return;
            };
            let Some(class) = d.get("class-code").and_then(plist_u32) else {
                return;
            };
            if vendor > u16::MAX as u32 || device > u16::MAX as u32 {
                return;
            }
            let section = match class >> 16 {
                3 => "GPU",
                2 => "Network",
                1 => "Storage Controllers",
                4 => "Sound",
                12 if class >> 8 == 0x0c03 => "USB Controllers",
                _ => "System Devices",
            };
            let id = format!("{vendor:04X}-{device:04X}");
            let mut name = d.get("model").map(plist_text).unwrap_or_default();
            if name.is_empty() {
                name = d
                    .get("IORegistryEntryName")
                    .map(plist_text)
                    .unwrap_or_else(|| format!("PCI {id}"));
            }
            let manufacturer = match vendor {
                0x8086 => "Intel",
                0x1002 => "AMD",
                0x10de => "NVIDIA",
                _ => "Unknown",
            };
            let mut entry = json!({"Device ID":id,"Manufacturer":manufacturer,"Bus Type":"PCI","PCI Class":format!("{class:06X}")});
            if section == "GPU" {
                let (code, kind, _) = compatibility::classify_gpu(&id);
                entry["Codename"] = code.into();
                entry["Device Type"] = kind.into();
            }
            count += 1;
            result[section][format!("{name} [{count}]")] = entry;
        })?;
    }
    if arm && let Some(displays) = profiler["SPDisplaysDataType"].as_array() {
        for (i, gpu) in displays.iter().enumerate() {
            result["GPU"][format!("{} [{i}]", value(gpu, "_name"))] = json!({"Manufacturer":"Apple","Codename":"Apple Silicon","Device Type":"Integrated GPU","Device ID":"Unknown"});
        }
    }
    let mut notes=vec!["macOS: PCI/ACPI routing may be incomplete; use the target PC's complete report before building EFI.".to_string()];
    if arm {
        notes.push("Apple Silicon is a preparation host; x86 OpenCore EFI requires a report and ACPI from the target Intel/AMD PC.".into());
    }
    if pci.is_none() {
        notes.push("IORegistry PCI data was unavailable.".into());
    }
    result["Scan Notes"] = json!(notes);
    Report::from_bytes(&serde_json::to_vec(&result)?)
}
fn valid_disk_id(id: &str) -> bool {
    id.strip_prefix("disk")
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}
pub fn list_disks() -> Result<Vec<Disk>> {
    let raw = command("/usr/sbin/diskutil", &["list", "-plist", "physical"])?;
    let plist = Plist::from_reader(Cursor::new(raw))?;
    let rows = plist
        .as_dictionary()
        .and_then(|d| d.get("AllDisksAndPartitions"))
        .and_then(Plist::as_array)
        .context("diskutil physical list")?;
    let mut result = vec![];
    for row in rows {
        let Some(id) = row
            .as_dictionary()
            .and_then(|d| d.get("DeviceIdentifier"))
            .and_then(Plist::as_string)
        else {
            continue;
        };
        if !valid_disk_id(id) {
            continue;
        }
        let raw = command("/usr/sbin/diskutil", &["info", "-plist", id])?;
        let info = Plist::from_reader(Cursor::new(raw))?;
        result.push(describe_disk(&info)?);
    }
    Ok(result)
}
pub fn describe_disk(info: &Plist) -> Result<Disk> {
    let d = info.as_dictionary().context("diskutil info dictionary")?;
    let s = |key: &str| d.get(key).and_then(Plist::as_string).unwrap_or("");
    let id = s("DeviceIdentifier");
    if !valid_disk_id(id) {
        bail!("Not a whole physical disk identifier");
    }
    let size = d
        .get("TotalSize")
        .and_then(Plist::as_unsigned_integer)
        .unwrap_or(0);
    let sector = d
        .get("DeviceBlockSize")
        .and_then(Plist::as_unsigned_integer)
        .unwrap_or(0);
    let mut reasons = vec![crate::i18n::tr("mac.usb_image_only").into()];
    if d.get("Internal").and_then(Plist::as_boolean) != Some(false) {
        reasons.push("Internal or unknown disk".into());
    }
    if d.get("WholeDisk").and_then(Plist::as_boolean) != Some(true) {
        reasons.push("Not a confirmed whole disk".into());
    }
    Ok(Disk {
        path: format!("/dev/{id}"),
        model: s("MediaName").into(),
        serial: String::new(),
        size,
        bus: s("BusProtocol").into(),
        reasons,
        device_id: s("DeviceTreePath").into(),
        sector_size: u32::try_from(sector).unwrap_or(0),
    })
}
pub fn capture_acpi(output: &Path) -> Result<usize> {
    let raw = command(
        "/usr/sbin/ioreg",
        &["-a", "-r", "-c", "AppleACPIPlatformExpert"],
    )?;
    let registry = Plist::from_reader(Cursor::new(raw))?;
    let tables = acpi_tables(&registry)?;
    if !tables.iter().any(|(_, raw)| raw.starts_with(b"DSDT")) {
        bail!(
            "DSDT is unavailable on this Mac. Import ACPI from the target Intel/AMD PC; Apple Silicon has no x86 ACPI."
        );
    }
    std::fs::create_dir_all(output)?;
    for (i, (name, raw)) in tables.iter().enumerate() {
        let file = output.join(format!("{name}-{i}.aml"));
        let mut out = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(file)?;
        use std::io::Write;
        out.write_all(raw)?;
        out.sync_all()?;
    }
    Ok(tables.len())
}
pub fn acpi_tables(registry: &Plist) -> Result<Vec<(String, Vec<u8>)>> {
    let mut tables = vec![];
    let mut error = None;
    visit(registry, 0, &mut |d| {
        if let Some(acpi) = d.get("ACPI Tables").and_then(Plist::as_dictionary) {
            for (name, value) in acpi {
                if let Some(raw) = value.as_data() {
                    match crate::acpi::validate_table(raw, name) {
                        Ok(table) => tables.push((table.signature, raw.to_vec())),
                        Err(e) => error = Some(e),
                    }
                }
            }
        }
    })?;
    if let Some(error) = error {
        return Err(error);
    }
    Ok(tables)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn little_endian_pci_data_is_not_reversed() {
        assert_eq!(
            plist_u32(&Plist::Data(vec![0x86, 0x80, 0, 0])),
            Some(0x8086)
        );
        assert_eq!(plist_u32(&Plist::Data(vec![1])), None);
    }
    #[test]
    fn apple_silicon_is_never_reported_as_intel() {
        let profiler = json!({"SPHardwareDataType":[{"chip_type":"Apple M3","machine_model":"MacBookPro18,1"}],"SPDisplaysDataType":[{"_name":"Apple M3"}]});
        let report = parse_report(&profiler, None, &BTreeMap::new()).unwrap();
        assert_eq!(report.hardware["CPU"]["Manufacturer"], "Apple");
        assert_eq!(
            report.hardware["CPU"]["SIMD Features"]
                .as_str()
                .unwrap()
                .trim(),
            ""
        );
        assert!(!crate::compatibility::analyze(&report, 24).ok);
    }
    #[test]
    fn disk_inventory_never_authorizes_writes() {
        let mut d = plist::Dictionary::new();
        d.insert("DeviceIdentifier".into(), "disk4".into());
        d.insert("WholeDisk".into(), true.into());
        d.insert("Internal".into(), false.into());
        let disk = describe_disk(&Plist::Dictionary(d)).unwrap();
        assert!(!disk.eligible());
        assert_eq!(disk.path, "/dev/disk4");
        assert!(!valid_disk_id("disk4s1"));
        assert!(!valid_disk_id("disk4;rm"));
    }
    #[test]
    fn absent_acpi_is_explicit() {
        assert!(acpi_tables(&Plist::Array(vec![])).unwrap().is_empty());
    }
}

#[cfg(test)]
mod fixture_tests {
    use super::*;
    #[test]
    fn intel_pci_fixture_keeps_hardware_ids() {
        let profiler =
            json!({"SPHardwareDataType":[{"machine_model":"iMac19,1","number_processors":"1"}]});
        let cpu = BTreeMap::from([
            ("machdep.cpu.vendor".into(), "GenuineIntel".into()),
            ("machdep.cpu.brand_string".into(), "Intel Core i7".into()),
            ("machdep.cpu.features".into(), "SSE4.1 SSE4.2".into()),
        ]);
        let mut gpu = plist::Dictionary::new();
        gpu.insert("vendor-id".into(), Plist::Data(vec![2, 16, 0, 0]));
        gpu.insert("device-id".into(), Plist::Data(vec![0xbf, 0x73, 0, 0]));
        gpu.insert("class-code".into(), Plist::Data(vec![0, 0, 3, 0]));
        let report = parse_report(
            &profiler,
            Some(&Plist::Array(vec![Plist::Dictionary(gpu)])),
            &cpu,
        )
        .unwrap();
        let rows = report.rows("GPU");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].1["Device ID"], "1002-73BF");
        assert_eq!(report.hardware["CPU"]["Manufacturer"], "Intel");
    }
    #[test]
    fn malformed_acpi_is_rejected_before_export() {
        let mut tables = plist::Dictionary::new();
        tables.insert("DSDT".into(), Plist::Data(vec![0; 8]));
        let mut root = plist::Dictionary::new();
        root.insert("ACPI Tables".into(), Plist::Dictionary(tables));
        assert!(acpi_tables(&Plist::Dictionary(root)).is_err());
    }
}
