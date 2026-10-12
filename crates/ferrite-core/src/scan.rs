use crate::report::Report;
#[cfg(target_os = "linux")]
use anyhow::Context;
use anyhow::Result;
#[cfg(target_os = "linux")]
use serde_json::Map;
#[cfg(target_os = "linux")]
use serde_json::{Value, json};
#[cfg(target_os = "linux")]
use std::{fs, path::Path};

/// Read-only scan. Missing evidence remains explicit instead of becoming a guessed value.
pub fn scan() -> Result<Report> {
    #[cfg(target_os = "linux")]
    {
        linux(Path::new("/sys"), Path::new("/proc"))
    }
    #[cfg(target_os = "windows")]
    {
        windows()
    }
    #[cfg(target_os = "macos")]
    {
        crate::macos::scan()
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
    {
        anyhow::bail!("Сканирование доступно на Windows и Linux")
    }
}
#[cfg(target_os = "linux")]
fn read(path: impl AsRef<Path>) -> String {
    fs::read_to_string(path)
        .unwrap_or_default()
        .trim()
        .to_string()
}
#[cfg(target_os = "linux")]
fn linux(sys: &Path, proc: &Path) -> Result<Report> {
    let cpu =
        fs::read_to_string(proc.join("cpuinfo")).context("Не удалось прочитать /proc/cpuinfo")?;
    let value = |key: &str| {
        cpu.lines()
            .find_map(|l| {
                l.split_once(':')
                    .filter(|(k, _)| k.trim() == key)
                    .map(|(_, v)| v.trim().to_string())
            })
            .unwrap_or_default()
    };
    let flags = value("flags")
        .replace("sse4_1", "SSE4.1")
        .replace("sse4_2", "SSE4.2")
        .replace("avx2", "AVX2");
    let family = value("cpu family");
    let model = value("model");
    let stepping = value("stepping");
    let identifier = format!("Family {family} Model {model} Stepping {stepping}");
    let code = crate::cpu::codename(&value("model name"), &identifier);
    let cores = value("cpu cores");
    let mut hardware = json!({"CPU":{"Processor Name":value("model name"),"Manufacturer":if value("vendor_id")=="GenuineIntel"{"Intel"}else if value("vendor_id")=="AuthenticAMD"{"AMD"}else{"Unknown"},"Codename":code,"Core Count":cores,"CPU Count":1,"SIMD Features":flags},"Motherboard":{"Name":read(sys.join("class/dmi/id/board_name")),"Manufacturer":read(sys.join("class/dmi/id/board_vendor")),"Platform":if sys.join("class/power_supply/BAT0").exists(){"Laptop"}else{"Unknown"}},"BIOS":{"Vendor":read(sys.join("class/dmi/id/bios_vendor")),"Version":read(sys.join("class/dmi/id/bios_version"))},"GPU":{},"Network":{},"Storage Controllers":{},"Sound":{},"Monitor":{},"Scan Notes":["Сканирование sysfs: ACPI, PCI Path для OpenCore и часть периферии требуют полного импортированного отчёта."]});
    let pci = fs::read_dir(sys.join("bus/pci/devices")).context("Нет доступа к PCI sysfs")?;
    for entry in pci {
        let entry = entry?;
        let path = entry.path();
        let class = read(path.join("class"));
        let vendor = read(path.join("vendor"))
            .trim_start_matches("0x")
            .to_uppercase();
        let device = read(path.join("device"))
            .trim_start_matches("0x")
            .to_uppercase();
        if vendor.len() != 4 || device.len() != 4 {
            continue;
        }
        let id = format!("{vendor}-{device}");
        let group = if class.starts_with("0x03") {
            "GPU"
        } else if class.starts_with("0x02") {
            "Network"
        } else if class.starts_with("0x01") {
            "Storage Controllers"
        } else if class.starts_with("0x0403") {
            "Sound"
        } else if class.starts_with("0x0c03") {
            "USB Controllers"
        } else {
            "System Devices"
        };
        let manufacturer = match vendor.as_str() {
            "8086" => "Intel",
            "1002" => "AMD",
            "10DE" => "NVIDIA",
            _ => "Unknown",
        };
        let address = entry.file_name().to_string_lossy().to_string();
        let name = format!("{manufacturer} {id} · {address}");
        let mut properties = json!({"Device ID":id,"Manufacturer":manufacturer,"Bus Type":"PCI","PCI Address":address,"Codename":"Unknown","Device Type":"Unknown"});
        if group == "GPU" {
            let (_, code, kind) = crate::compatibility::classify_gpu(&id);
            properties["Codename"] = json!(code);
            properties["Device Type"] = json!(kind);
        }
        let sv = read(path.join("subsystem_vendor"))
            .trim_start_matches("0x")
            .to_uppercase();
        let sd = read(path.join("subsystem_device"))
            .trim_start_matches("0x")
            .to_uppercase();
        properties["Subsystem ID"] = json!(format!("{sd}{sv}"));
        let acpi_path = read(path.join("firmware_node/path"));
        if !acpi_path.is_empty() {
            properties["ACPI Path"] = json!(acpi_path);
        }
        let canonical = fs::canonicalize(&path)?;
        let segments: Vec<_> = canonical
            .components()
            .filter_map(|c| crate::cpu::pci_segment(&c.as_os_str().to_string_lossy()))
            .collect();
        if let Some(root) = canonical.ancestors().find(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("pci"))
        }) {
            let uid = read(root.join("firmware_node/uid"));
            if let Ok(uid) = uid.parse::<u64>() {
                properties["PCI Path"] = json!(format!("PciRoot(0x{uid:X}){}", segments.join("")));
            }
        }

        if group == "Storage Controllers" {
            let canonical = fs::canonicalize(&path)?;
            let mut drives = Map::new();
            if let Ok(blocks) = fs::read_dir(sys.join("class/block")) {
                for block in blocks.flatten() {
                    let bp = block.path();
                    if bp.join("partition").exists() {
                        continue;
                    }
                    if fs::canonicalize(bp.join("device")).is_ok_and(|d| d.starts_with(&canonical))
                    {
                        let n = block.file_name().to_string_lossy().to_string();
                        drives.insert(n, json!({"Model":read(bp.join("device/model"))}));
                    }
                }
            }
            properties["Disk Drives"] = Value::Object(drives);
        }
        hardware
            .as_object_mut()
            .unwrap()
            .entry(group)
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .unwrap()
            .insert(name, properties);
    }
    // DRM connectors provide actual routing to PCI devices where exposed by sysfs.
    if let Ok(entries) = fs::read_dir(sys.join("class/drm")) {
        for entry in entries.flatten() {
            let p = entry.path();
            if read(p.join("status")) != "connected" {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            let card = name.split('-').next().unwrap_or("");
            let pci_path = fs::canonicalize(sys.join("class/drm").join(card).join("device")).ok();
            let gpu = hardware["GPU"]
                .as_object()
                .and_then(|rows| {
                    rows.iter().find(|(_, v)| {
                        pci_path
                            .as_ref()
                            .is_some_and(|p| p.file_name().is_some_and(|f| f == text_address(v)))
                    })
                })
                .map(|(n, _)| n.clone())
                .unwrap_or_default();
            let internal = name.contains("eDP") || name.contains("LVDS");
            hardware["Monitor"].as_object_mut().unwrap().insert(name,json!({"Connector Type":if internal {"Internal"}else{"External"},"Connected GPU":gpu}));
        }
    }
    Report::from_bytes(&serde_json::to_vec(&hardware)?)
}
#[cfg(target_os = "linux")]
fn text_address(v: &Value) -> &str {
    v["PCI Address"].as_str().unwrap_or("")
}

#[cfg(target_os = "windows")]
fn windows() -> Result<Report> {
    crate::windows_scan::scan()
}
