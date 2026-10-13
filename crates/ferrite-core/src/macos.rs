//! macOS inventory and hardware adapters; host-independent safety parsers run on every platform.
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
pub(crate) fn valid_disk_id(id: &str) -> bool {
    id.strip_prefix("disk")
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}
#[derive(Clone, Debug)]
pub struct MediaIdentity {
    pub registry_id: u64,
    pub usb: bool,
    pub serial: String,
}

#[cfg(any(target_os = "macos", test))]
pub(crate) fn disk_path_id(path: &str) -> Result<&str> {
    let id = path.strip_prefix("/dev/").context("Expected /dev/diskN")?;
    if !valid_disk_id(id) {
        bail!("Выберите весь накопитель /dev/diskN, а не раздел или raw/symlink путь");
    }
    Ok(id)
}

fn whole_id(id: &str) -> Result<String> {
    let mut parts = id
        .strip_prefix("disk")
        .context("Disk identifier")?
        .split('s');
    let whole = format!("disk{}", parts.next().context("Disk identifier")?);
    if !valid_disk_id(&whole)
        || parts.any(|p| p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()))
    {
        bail!("Invalid disk identifier");
    }
    Ok(whole)
}

pub fn protected_disks(root: &Plist) -> Result<Vec<String>> {
    let d = root.as_dictionary().context("Root disk metadata")?;
    let mut ids = vec![whole_id(
        d.get("ParentWholeDisk")
            .and_then(Plist::as_string)
            .context("Root whole disk is unknown")?,
    )?];
    if d.get("FilesystemType").and_then(Plist::as_string) == Some("apfs") {
        let stores = d
            .get("APFSPhysicalStores")
            .and_then(Plist::as_array)
            .context("Root APFS physical stores are unknown")?;
        if stores.is_empty() {
            bail!("Root APFS has no physical stores");
        }
        for store in stores {
            let id = store
                .as_dictionary()
                .and_then(|x| x.get("APFSPhysicalStore"))
                .and_then(Plist::as_string)
                .context("Root APFS store is unknown")?;
            ids.push(whole_id(id)?);
        }
    } else if !matches!(
        d.get("FilesystemType").and_then(Plist::as_string),
        Some("hfs" | "hfs+")
    ) {
        bail!("Unknown root filesystem; cannot authorize disk writes");
    }
    for key in ["BooterDeviceIdentifier", "RecoveryDeviceIdentifier"] {
        if let Some(id) = d.get(key).and_then(Plist::as_string)
            && !id.is_empty()
        {
            ids.push(whole_id(id)?);
        }
    }
    ids.sort();
    ids.dedup();
    Ok(ids)
}

pub fn list_disks() -> Result<Vec<Disk>> {
    let raw = command("/usr/sbin/diskutil", &["info", "-plist", "/"])?;
    let protected = protected_disks(&Plist::from_reader(Cursor::new(raw))?)?;
    let raw = command("/usr/sbin/diskutil", &["list", "-plist", "physical"])?;
    let plist = Plist::from_reader(Cursor::new(raw))?;
    let rows = plist
        .as_dictionary()
        .and_then(|d| d.get("AllDisksAndPartitions"))
        .and_then(Plist::as_array)
        .context("diskutil physical list")?;
    let mut result = vec![];
    for row in rows {
        let d = row.as_dictionary().context("Disk layout")?;
        let id = d
            .get("DeviceIdentifier")
            .and_then(Plist::as_string)
            .context("Disk layout id")?;
        if !valid_disk_id(id) {
            bail!("Physical inventory included a non-whole disk");
        }
        let raw = command("/usr/sbin/diskutil", &["info", "-plist", id])?;
        let info = Plist::from_reader(Cursor::new(raw))?;
        let mut partitions = vec![];
        if let Some(items) = d.get("Partitions").and_then(Plist::as_array) {
            for part in items {
                let pid = part
                    .as_dictionary()
                    .and_then(|p| p.get("DeviceIdentifier"))
                    .and_then(Plist::as_string)
                    .context("Partition id")?;
                if whole_id(pid)? != id || pid == id {
                    bail!("Partition changed parent");
                }
                let raw = command("/usr/sbin/diskutil", &["info", "-plist", pid])?;
                partitions.push(Plist::from_reader(Cursor::new(raw))?);
            }
        }
        #[cfg(target_os = "macos")]
        let identity = crate::macos_disk::identity(id).ok();
        #[cfg(not(target_os = "macos"))]
        let identity = None;
        result.push(classify_disk(
            &info,
            row,
            &partitions,
            &protected,
            identity.as_ref(),
        )?);
    }
    Ok(result)
}

pub fn classify_disk(
    info: &Plist,
    layout: &Plist,
    partitions: &[Plist],
    protected: &[String],
    identity: Option<&MediaIdentity>,
) -> Result<Disk> {
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
    let mut reasons = vec![];
    if d.get("Internal").and_then(Plist::as_boolean) != Some(false) {
        reasons.push("Internal or unknown disk".into());
    }
    if d.get("WholeDisk").and_then(Plist::as_boolean) != Some(true) {
        reasons.push("Not a confirmed whole disk".into());
    }
    if s("BusProtocol") != "USB" || !identity.is_some_and(|i| i.usb) {
        reasons.push("Not a confirmed physical USB device".into());
    }
    if d.get("Writable").and_then(Plist::as_boolean) != Some(true)
        || d.get("WritableMedia").and_then(Plist::as_boolean) != Some(true)
    {
        reasons.push("Read-only or unknown writable state".into());
    }
    if d.get("Virtual").and_then(Plist::as_boolean) == Some(true)
        || matches!(s("VirtualOrPhysical"), "Virtual")
    {
        reasons.push("Virtual disk cannot be erased by the USB writer".into());
    }
    if protected.iter().any(|x| x == id) {
        reasons.push("System/boot/APFS physical store".into());
    }
    if !(8 * 1024 * 1024 * 1024..=128 * 1024 * 1024 * 1024).contains(&size) {
        reasons.push("USB must be 8–128 GiB".into());
    }
    if sector != 512 {
        reasons.push("USB logical sector must be 512 bytes".into());
    }
    if s("MediaName").is_empty() {
        reasons.push("Unknown disk model".into());
    }
    if !identity.is_some_and(|i| i.registry_id != 0) {
        reasons.push("IOKit identity unavailable".into());
    }
    let row = layout.as_dictionary().context("Physical layout")?;
    if row.get("DeviceIdentifier").and_then(Plist::as_string) != Some(id) {
        bail!("Layout identity changed");
    }
    if let Some(parts) = row.get("Partitions").and_then(Plist::as_array) {
        if parts.len() != partitions.len() {
            bail!("Partition inventory incomplete");
        }
        for (part, observed) in parts.iter().zip(partitions) {
            let expected = part
                .as_dictionary()
                .and_then(|p| p.get("DeviceIdentifier"))
                .and_then(Plist::as_string)
                .context("Partition identity missing")?;
            let observed = observed.as_dictionary().context("Partition info")?;
            if whole_id(expected)? != id
                || observed.get("DeviceIdentifier").and_then(Plist::as_string) != Some(expected)
            {
                bail!("Partition inventory changed");
            }
            if let Some(parent) = observed.get("ParentWholeDisk").and_then(Plist::as_string)
                && parent != id
            {
                bail!("Partition changed parent");
            }
            let content = part
                .as_dictionary()
                .and_then(|p| p.get("Content"))
                .and_then(Plist::as_string)
                .unwrap_or("");
            if [
                "Apple_APFS",
                "Apple_HFS",
                "Apple_CoreStorage",
                "Apple_Boot",
                "Apple_APFS_ISC",
                "Apple_APFS_Recovery",
                "Linux Filesystem",
                "Linux LVM",
                "Linux RAID",
                "Linux Swap",
                "Windows_NTFS",
                "Windows Recovery",
            ]
            .contains(&content)
            {
                reasons.push("OS, recovery, backup or encrypted partition".into());
            }
        }
    }
    for p in partitions.iter().chain([info]) {
        let p = p.as_dictionary().context("Partition info")?;
        if let Some(fs) = p.get("FilesystemType").and_then(Plist::as_string)
            && !["msdos", "exfat"].contains(&fs.to_lowercase().as_str())
        {
            reasons.push("OS/unsupported filesystem".into());
        }
        if let Some(mount) = p.get("MountPoint").and_then(Plist::as_string)
            && !mount.is_empty()
            && !mount.starts_with("/Volumes/")
        {
            reasons.push("Protected mount point".into());
        }
    }
    reasons.sort();
    reasons.dedup();
    Ok(Disk {
        path: format!("/dev/{id}"),
        model: s("MediaName").into(),
        serial: identity
            .map(|i| {
                if i.serial.is_empty() {
                    format!("connection:{}", i.registry_id)
                } else {
                    i.serial.clone()
                }
            })
            .unwrap_or_default(),
        size,
        bus: s("BusProtocol").into(),
        reasons,
        device_id: identity
            .map(|i| format!("ioreg:{}", i.registry_id))
            .unwrap_or_default(),
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
        let info = Plist::Dictionary(d);
        let disk = classify_disk(&info, &info, &[], &[], None).unwrap();
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

    fn usb_info() -> Plist {
        plist::to_value(&json!({"DeviceIdentifier":"disk4", "WholeDisk":true,
            "Internal":false,"Writable":true,"WritableMedia":true,"BusProtocol":"USB",
            "MediaName":"Test USB","TotalSize":17179869184u64,"DeviceBlockSize":512}))
        .unwrap()
    }
    #[test]
    fn mac_usb_requires_native_identity_and_known_writable_media() {
        let info = usb_info();
        let layout = plist::to_value(&json!({"DeviceIdentifier":"disk4","Partitions":[]})).unwrap();
        let identity = MediaIdentity {
            registry_id: 42,
            usb: true,
            serial: String::new(),
        };
        let allowed = classify_disk(&info, &layout, &[], &[], Some(&identity)).unwrap();
        assert!(allowed.eligible(), "{:?}", allowed.reasons);
        assert_eq!(allowed.serial, "connection:42");
        assert!(
            !classify_disk(&info, &layout, &[], &[], None)
                .unwrap()
                .eligible()
        );
        assert!(
            !classify_disk(&info, &layout, &[], &["disk4".into()], Some(&identity))
                .unwrap()
                .eligible()
        );
        for key in ["Internal", "WholeDisk", "Writable", "WritableMedia"] {
            let mut bad = info.clone();
            bad.as_dictionary_mut().unwrap().remove(key);
            assert!(
                !classify_disk(&bad, &layout, &[], &[], Some(&identity))
                    .unwrap()
                    .eligible()
            );
        }
        for path in [
            "/dev/rdisk4",
            "/dev/disk4s1",
            "/dev/disk4/../disk0",
            "/tmp/disk4",
            "/dev/disk4;id",
        ] {
            assert!(disk_path_id(path).is_err());
        }
    }
    #[test]
    fn root_apfs_resolves_all_physical_stores_and_fails_closed() {
        let root=plist::to_value(&json!({"ParentWholeDisk":"disk3","FilesystemType":"apfs",
            "APFSPhysicalStores":[{"APFSPhysicalStore":"disk0s2"},{"APFSPhysicalStore":"disk1s2"}]})).unwrap();
        assert_eq!(
            protected_disks(&root).unwrap(),
            vec!["disk0", "disk1", "disk3"]
        );
        let mut bad = root.clone();
        bad.as_dictionary_mut()
            .unwrap()
            .remove("APFSPhysicalStores");
        assert!(protected_disks(&bad).is_err());
        assert!(whole_id("disk0s2s1").is_ok());
        assert!(whole_id("disk0s").is_err());
    }
    #[test]
    fn nested_os_partitions_and_incomplete_inventory_are_blocked() {
        let info = usb_info();
        let layout=plist::to_value(&json!({"DeviceIdentifier":"disk4","Partitions":[{"DeviceIdentifier":"disk4s1","Content":"Apple_APFS"}]})).unwrap();
        let identity = MediaIdentity {
            registry_id: 42,
            usb: true,
            serial: "SERIAL".into(),
        };
        let part=plist::to_value(&json!({"DeviceIdentifier":"disk4s1","FilesystemType":"apfs","MountPoint":"/Volumes/Backup"})).unwrap();
        assert!(
            !classify_disk(
                &info,
                &layout,
                std::slice::from_ref(&part),
                &[],
                Some(&identity)
            )
            .unwrap()
            .eligible()
        );
        assert!(classify_disk(&info, &layout, &[], &[], Some(&identity)).is_err());
    }
}
