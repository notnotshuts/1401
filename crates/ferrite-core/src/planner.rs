use crate::{
    compatibility::{self, Status},
    downloads::{self, Artifact},
    report::{Report, text},
};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::LazyLock,
};
static KEXTS: LazyLock<Vec<Kext>> =
    LazyLock::new(|| serde_json::from_str(include_str!("../data/kexts.json")).expect("kext data"));
static PCI: LazyLock<Value> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../data/pci-rules.json")).expect("pci data")
});
static PLATFORMS: LazyLock<Value> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../data/platforms.json")).expect("platform data")
});
static LAYOUTS: LazyLock<Value> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../data/codec-layouts.json")).expect("audio data")
});
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Kext {
    pub name: String,
    pub required: bool,
    pub min: String,
    pub max: String,
    pub requires: Vec<String>,
    pub conflict: Option<String>,
    pub repo: Value,
    pub download: Value,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Options {
    pub target: u8,
    pub smbios: Option<String>,
    pub audio_layout: Option<u64>,
    pub intel_wifi: String,
    pub verbose: bool,
    pub extra_kexts: Vec<String>,
    pub disabled: Vec<String>,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            target: 24,
            smbios: None,
            audio_layout: None,
            intel_wifi: "auto".into(),
            verbose: true,
            extra_kexts: vec![],
            disabled: vec![],
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Plan {
    pub report_sha256: String,
    pub target: u8,
    pub smbios: String,
    pub kexts: Vec<Kext>,
    pub artifacts: Vec<Artifact>,
    pub disabled: Vec<String>,
    pub needs_oclp: bool,
    pub nvidia: bool,
    pub audio_layout: Option<u64>,
    pub notices: Vec<String>,
    pub hardware: Value,
    pub decisions: Vec<String>,
}
pub fn table(name: &str, id: &str) -> bool {
    PCI[name]
        .as_array()
        .is_some_and(|a| a.iter().any(|v| v.as_str() == Some(id)))
}
pub fn index(name: &str, id: &str) -> Option<usize> {
    PCI[name]
        .as_array()?
        .iter()
        .position(|v| v.as_str() == Some(id))
}
pub fn platform_index(name: &str, value: &str) -> Option<usize> {
    PLATFORMS[name]
        .as_array()?
        .iter()
        .position(|v| v.as_str() == Some(value))
}
fn major(version: &str) -> u8 {
    version
        .split('.')
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}
pub fn is_hedt(r: &Report) -> bool {
    let code = r.text("CPU", "Codename");
    let idx = platform_index("IntelCPUGenerations", code);
    idx.is_some_and(|i| {
        i >= 45
            && ((i < 66
                && ["-X", "-P", "-W", "-E", "-EP", "-EX"]
                    .iter()
                    .any(|x| code.ends_with(x)))
                || (i >= 66 && r.text("CPU", "Processor Name").contains("Xeon")))
    })
}
pub fn number(v: &Value, key: &str) -> Option<u64> {
    v[key].as_u64().or_else(|| v[key].as_str()?.parse().ok())
}
pub fn models() -> Value {
    serde_json::from_str(include_str!("../data/models.json")).expect("models")
}
pub fn select_smbios(r: &Report, target: u8) -> String {
    let platform = r.text("Motherboard", "Platform");
    let code = r.text("CPU", "Codename");
    let name = r.text("CPU", "Processor Name");
    let cores = number(&r.hardware["CPU"], "Core Count").unwrap_or(4);
    let laptop = platform == "Laptop";
    let nuc = r.text("Motherboard", "Name").contains("NUC");
    let has_igpu = r
        .rows("GPU")
        .iter()
        .any(|(_, g)| text(g, "Device Type") == "Integrated GPU");
    let default = if laptop {
        "MacBookPro16,2"
    } else if target >= 25 {
        "MacPro7,1"
    } else {
        "iMacPro1,1"
    };
    if !laptop && !has_igpu {
        return default.into();
    }
    let model = if code.contains("Sandy Bridge") {
        if laptop {
            if cores < 4 {
                "MacBookPro8,1"
            } else {
                "MacBookPro8,2"
            }
        } else if nuc {
            if cores < 4 {
                "Macmini5,1"
            } else {
                "Macmini5,3"
            }
        } else {
            "iMac12,2"
        }
    } else if code.contains("Ivy Bridge") {
        if laptop {
            if cores < 4 {
                "MacBookPro10,2"
            } else {
                "MacBookPro10,1"
            }
        } else if nuc {
            if cores < 4 {
                "Macmini6,1"
            } else {
                "Macmini6,2"
            }
        } else {
            "iMac13,1"
        }
    } else if code.contains("Haswell") {
        if laptop {
            if cores < 4 {
                "MacBookPro11,1"
            } else {
                "MacBookPro11,5"
            }
        } else if nuc {
            "Macmini7,1"
        } else {
            "iMac14,4"
        }
    } else if code.contains("Broadwell") {
        if laptop {
            if cores < 4 {
                "MacBookPro12,1"
            } else {
                "MacBookPro11,5"
            }
        } else if nuc {
            "iMac16,1"
        } else {
            "iMac16,2"
        }
    } else if code.contains("Skylake") {
        if laptop {
            if cores < 4 {
                "MacBookPro13,1"
            } else {
                "MacBookPro13,3"
            }
        } else {
            "iMac17,1"
        }
    } else if code.contains("Kaby Lake") || code.contains("Amber Lake") {
        if laptop {
            if cores < 4 {
                "MacBookPro14,1"
            } else {
                "MacBookPro14,3"
            }
        } else {
            "iMac18,1"
        }
    } else if ["Cannon Lake", "Whiskey Lake", "Coffee Lake", "Comet Lake"]
        .iter()
        .any(|c| code.contains(c))
    {
        if laptop {
            if name.contains("-8") {
                if cores < 6 {
                    "MacBookPro15,2"
                } else {
                    "MacBookPro15,3"
                }
            } else if cores < 6 {
                "MacBookPro16,3"
            } else {
                "MacBookPro16,1"
            }
        } else if nuc {
            "Macmini8,1"
        } else if code.contains("Comet Lake") {
            if cores < 10 { "iMac20,1" } else { "iMac20,2" }
        } else {
            "iMac19,1"
        }
    } else if code.contains("Ice Lake") {
        "MacBookAir9,1"
    } else {
        default
    };
    model.into()
}
pub fn resolve(names: &BTreeSet<String>, target: u8) -> Result<Vec<Kext>> {
    let map: BTreeMap<_, _> = KEXTS.iter().map(|k| (k.name.as_str(), k)).collect();
    let mut result = vec![];
    let mut visiting = BTreeSet::new();
    let mut visited = BTreeSet::new();
    let mut conflicts = BTreeMap::new();
    fn visit(
        name: &str,
        target: u8,
        map: &BTreeMap<&str, &Kext>,
        visiting: &mut BTreeSet<String>,
        visited: &mut BTreeSet<String>,
        result: &mut Vec<Kext>,
        conflicts: &mut BTreeMap<String, String>,
    ) -> Result<()> {
        if visited.contains(name) {
            return Ok(());
        }
        if !visiting.insert(name.into()) {
            bail!("Цикл зависимостей kext: {name}");
        }
        let k = map
            .get(name)
            .with_context(|| format!("Неизвестный kext: {name}"))?;
        if target < major(&k.min) || target > major(&k.max) {
            bail!("{name} вне закреплённого диапазона {}–{}", k.min, k.max);
        }
        if let Some(group) = &k.conflict
            && let Some(other) = conflicts.insert(group.clone(), k.name.clone())
            && other != k.name
        {
            bail!("Несовместимые kexts: {other} и {name}");
        }
        for dep in &k.requires {
            visit(dep, target, map, visiting, visited, result, conflicts)?;
        }
        visiting.remove(name);
        visited.insert(name.into());
        result.push((*k).clone());
        Ok(())
    }
    for name in names {
        visit(
            name,
            target,
            &map,
            &mut visiting,
            &mut visited,
            &mut result,
            &mut conflicts,
        )?;
    }
    Ok(result)
}
pub fn kext_artifact(k: &Kext, target: u8) -> Result<Artifact> {
    if let Some(url) = k.download["url"].as_str()
        && let Ok(a) = downloads::artifact(&k.name, url)
    {
        return Ok(a);
    }
    if let Some(repo) = k.repo["repo"].as_str()
        && let Ok(a) = downloads::product(repo)
    {
        return Ok(a);
    }
    // Releases not present in Dortania build-repo are resolved only within the pinned manifest.
    let manifest: Value = serde_json::from_str(include_str!("../data/downloads.json"))?;
    let owner = k.repo["owner"].as_str().unwrap_or("");
    let repo = k.repo["repo"].as_str().unwrap_or("");
    let prefix = format!("https://github.com/{owner}/{repo}/releases/download/");
    let candidates: Vec<_> = manifest["entries"]
        .as_object()
        .context("manifest")?
        .keys()
        .filter(|u| u.starts_with(&prefix) && u.ends_with(".zip"))
        .collect();
    let wanted = if k.name == "AirportItlwm" {
        format!(
            "{}_",
            compatibility::version_name(target)
                .split(' ')
                .next()
                .unwrap_or("")
        )
    } else {
        k.name.clone()
    };
    if let Some(url) = candidates.iter().find(|u| {
        if k.name == "itlwm" {
            u.rsplit('/')
                .next()
                .is_some_and(|n| n.starts_with("itlwm-"))
        } else {
            u.to_lowercase().contains(&wanted.to_lowercase())
        }
    }) {
        return downloads::artifact(&k.name, url);
    }
    if candidates.len() == 1 {
        return downloads::artifact(&k.name, candidates[0]);
    }
    bail!(
        "Нет однозначного закреплённого архива {}/{} ({})",
        owner,
        repo,
        k.name
    )
}
pub fn plan(report: &Report, options: &Options) -> Result<Plan> {
    let mut report = report.clone();
    let target = options.target;
    let mut disabled = options.disabled.clone();
    for section in [
        "GPU",
        "Network",
        "Sound",
        "Bluetooth",
        "Storage Controllers",
        "SD Controller",
    ] {
        if let Some(rows) = report.hardware[section].as_object_mut() {
            rows.retain(|name, _| !disabled.contains(name));
        }
    }
    let analysis = compatibility::analyze(&report, target);
    if !analysis.ok {
        bail!("Сборка заблокирована: {}", analysis.blockers.join("; "));
    }
    // Pick one Wi-Fi card using newest native support; disabled optional devices cannot demand OCLP.
    let wifi: Vec<_> = analysis
        .devices
        .iter()
        .filter(|d| d.section == "Network" && table("WirelessCardIDs", &d.id))
        .collect();
    let chosen = wifi
        .iter()
        .filter(|d| matches!(d.status, Status::Supported | Status::Patched))
        .max_by_key(|d| {
            (
                d.status == Status::Supported,
                d.native.map(|r| r.max).unwrap_or((0, 0, 0)),
            )
        })
        .map(|d| d.name.clone());
    if let Some(selected) = &chosen {
        for w in &wifi {
            if &w.name != selected {
                disabled.push(w.name.clone());
            }
        }
    }
    for d in &analysis.devices {
        if d.status == Status::Unsupported
            && d.section != "CPU"
            && d.section != "Storage Controllers"
        {
            disabled.push(d.name.clone());
        }
    }
    disabled.sort();
    disabled.dedup();
    let live = |section: &str| {
        report
            .rows(section)
            .into_iter()
            .filter(|(n, _)| !disabled.iter().any(|d| d == n))
            .collect::<Vec<_>>()
    };
    let cpu = report.text("CPU", "Manufacturer");
    let code = report.text("CPU", "Codename");
    let board = report.text("Motherboard", "Name").to_uppercase();
    let laptop = report.text("Motherboard", "Platform") == "Laptop";
    let amd = cpu.contains("AMD");
    let intel = cpu.contains("Intel");
    let cpu_index = platform_index("IntelCPUGenerations", code);
    let mut names: BTreeSet<String> = KEXTS
        .iter()
        .filter(|k| k.required)
        .map(|k| k.name.clone())
        .collect();
    let mut add = |name: &str| {
        names.insert(name.into());
    };
    add("UTBDefault");
    if intel {
        add("SMCProcessor");
        add("SMCSuperIO");
    }
    if laptop && !board.contains("SURFACE") {
        add("SMCBatteryManager");
        add("SMCLightSensor");
        if board.contains("DELL") {
            add("SMCDellSensors");
        }
    }
    if target >= 23 || amd || cpu_index.is_none_or(|i| i < 28) {
        add("RestrictEvents");
    }
    if (amd && target >= 21)
        || number(&report.hardware["CPU"], "CPU Count").is_some_and(|n| n > 1)
        || cpu_index.is_some_and(|i| i < 4)
    {
        add("AppleMCEReporterDisabler");
    }
    if target >= 22 && !report.text("CPU", "SIMD Features").contains("AVX2") {
        add("CryptexFixup");
    }
    if intel
        && cpu_index.is_some_and(|i| i < 20)
        && number(&report.hardware["CPU"], "Core Count").is_some_and(|n| n > 6)
        && !code.contains("Arrow Lake")
        && !code.contains("Meteor Lake")
        && !code.contains("Lunar Lake")
    {
        add("CpuTopologyRebuild");
    }
    let needs_oclp = analysis
        .devices
        .iter()
        .any(|d| d.status == Status::Patched && !disabled.contains(&d.name));
    if needs_oclp {
        add("AMFIPass");
        add("RestrictEvents");
    }
    let nvidia = analysis
        .devices
        .iter()
        .any(|d| d.status == Status::Driver && !disabled.contains(&d.name));
    let mut nooted = false;
    let mut nootrx = false;
    let mut weg = false;
    let mut notices = analysis.warnings.clone();
    for (_, g) in live("GPU") {
        let c = text(g, "Codename");
        if text(g, "Manufacturer") == "AMD" && text(g, "Device Type") == "Integrated GPU" {
            nooted = true;
        } else if c == "Navi 22" {
            nootrx = true;
        } else if !(nvidia && text(g, "Manufacturer") == "NVIDIA") {
            weg = true;
        }
    }
    if nooted {
        add("NootedRed");
        if laptop {
            add("ForgedInvariant");
        }
    } else if nootrx {
        add("NootRX");
    } else if weg {
        if target == 25
            && live("GPU")
                .iter()
                .all(|(_, g)| text(g, "Manufacturer") == "AMD")
        {
            notices.push(
                "Tahoe: WhateverGreen для AMD не загружается; pikera переносится в kernel patch"
                    .into(),
            );
        } else {
            add("WhateverGreen");
        }
    }
    if is_hedt(&report) {
        add("CpuTscSync");
    }
    if laptop && board.contains("ASUS") {
        add("ForgedInvariant");
        add("AsusSMC");
    }
    let mut layout = None;
    for (_, sound) in live("Sound") {
        if let Some(layouts) = LAYOUTS[text(sound, "Device ID")].as_array() {
            layout = options
                .audio_layout
                .or_else(|| layouts.first()?.get("id")?.as_u64());
            if let Some(id) = layout
                && !layouts.iter().any(|l| l["id"].as_u64() == Some(id))
            {
                bail!("Audio layout {id} отсутствует в таблице кодека");
            }
            if target == 25 {
                notices.push(
                    "Tahoe: требуется отдельный rollback AppleHDA через OCLP для AppleALC".into(),
                );
            }
            add("AppleALC");
            break;
        }
    }
    for (_, network) in live("Network") {
        let id = text(network, "Device ID");
        if table("BroadcomWiFiIDs", id) {
            if target >= 23 {
                add("IOSkywalkFamily");
            }
            if index("BroadcomWiFiIDs", id).is_some_and(|i| i < 18) {
                add("AirportBrcmFixup");
            }
        } else if table("IntelWiFiIDs", id) {
            let stable =
                options.intel_wifi == "itlwm" || (options.intel_wifi == "auto" && target >= 23);
            if stable {
                add("itlwm");
                notices.push("Intel Wi-Fi: установите HeliPort в macOS; itlwm не использует штатное меню Wi-Fi".into());
            } else {
                add("AirportItlwm");
                if target >= 24 {
                    add("IOSkywalkFamily");
                }
            }
        } else if index("AtherosWiFiIDs", id).is_some_and(|i| i < 8) {
            add("corecaptureElCap");
            if target >= 21 {
                add("AMFIPass");
            }
        } else {
            for (table_name, kext) in [
                ("rtw88WiFiIDs", "Feixiao"),
                ("IntelI22XIDs", "AppleIGC"),
                ("AtherosE2200IDs", "AtherosE2200Ethernet"),
                ("IntelMausiIDs", "IntelMausiEthernet"),
                ("RealtekRTL8100IDs", "RealtekRTL8100"),
                ("RealtekRTL8111IDs", "RealtekRTL8111"),
                ("AppleIGBIDs", "AppleIGB"),
                ("BroadcomBCM57XXIDs", "CatalinaBCM5701Ethernet"),
                ("IntelX500IDs", "IntelLucy"),
                ("RealtekRTL812xIDs", "RTL812xLucy"),
            ] {
                if table(table_name, id) {
                    add(kext);
                    break;
                }
            }
        }
    }
    if live("Network")
        .iter()
        .all(|(_, n)| text(n, "Bus Type") == "USB")
    {
        add("NullEthernet");
    }
    for (_, b) in live("Bluetooth") {
        let id = text(b, "Device ID");
        let group = if table("IntelBluetoothIDs", id) {
            if target >= 21 {
                vec!["IntelBluetoothFirmware", "IntelBTPatcher", "BlueToolFixup"]
            } else {
                vec!["IntelBluetoothFirmware", "IntelBluetoothInjector"]
            }
        } else if table("BroadcomBluetoothIDs", id) {
            if target >= 21 {
                vec!["BrcmFirmwareData", "BrcmPatchRAM3", "BlueToolFixup"]
            } else if target >= 19 {
                vec!["BrcmFirmwareData", "BrcmPatchRAM3", "BrcmBluetoothInjector"]
            } else {
                vec!["BrcmFirmwareData", "BrcmPatchRAM2", "BrcmBluetoothInjector"]
            }
        } else if table("AtherosBluetoothIDs", id) {
            vec!["Ath3kBT", "Ath3kBTInjector"]
        } else if table("RealtekBluetoothIDs", id) {
            vec!["RealtekBluetoothFirmware", "BlueToolFixup"]
        } else {
            vec![]
        };
        for k in group {
            add(k);
        }
    }
    if laptop {
        if board.contains("SURFACE") {
            add("BigSurface");
        } else {
            add("BrightnessKeys");
            for (_, i) in live("Input") {
                let kind = text(i, "Device Type");
                let id = text(i, "Device");
                if kind.contains("PS/2") {
                    add("VoodooPS2Controller");
                    if id.starts_with("SYN") {
                        add("VoodooRMI");
                    }
                }
                if kind.contains("I2C") {
                    add("VoodooI2CHID");
                }
            }
        }
    }
    for (_, s) in live("SD Controller") {
        if let Some(i) = index("RealtekCardReaderIDs", text(s, "Device ID")) {
            add(if i < 5 {
                "RealtekCardReader"
            } else {
                "Sinetek-rtsx"
            });
        }
    }
    for (name, s) in live("Storage Controllers") {
        let id = text(s, "Device ID");
        if id == "1C5C-174A" {
            add("PC711Probe");
        }
        if name.contains("NVMe") || name.contains("NVM Express") {
            add("NVMeFix");
        }
        if table("UnsupportedSATAControllerIDs", id) {
            add(if target >= 20 {
                "CtlnaAHCIPort"
            } else {
                "SATA-unsupported"
            });
        }
    }
    if code.contains("Sandy Bridge") {
        add("ASPP-Override");
    }
    if code.contains("Sandy Bridge") || code.contains("Ivy Bridge") {
        add("AppleIntelCPUPowerManagement");
        add("AppleIntelCPUPowerManagementClient");
    }
    for name in &options.extra_kexts {
        add(name);
    }
    let kexts = resolve(&names, target)?;
    let mut artifacts = vec![downloads::product("OpenCorePkg")?];
    let mut urls = BTreeSet::new();
    for k in &kexts {
        let a = kext_artifact(k, target)?;
        if urls.insert(a.url.clone()) {
            artifacts.push(a);
        }
    }
    artifacts.push(downloads::artifact(
        "OcBinaryData",
        "https://github.com/acidanthera/OcBinaryData/archive/refs/heads/master.zip",
    )?);
    if amd {
        artifacts.push(downloads::artifact("AMD Vanilla Patches","https://raw.githubusercontent.com/laobamac/AMD_Vanilla/refs/heads/master/patches.plist")?);
    }
    let model = options
        .smbios
        .clone()
        .unwrap_or_else(|| select_smbios(&report, target));
    let models = models();
    let device = models
        .as_array()
        .context("models")?
        .iter()
        .find(|m| m["name"] == model)
        .context("Неизвестный SMBIOS model")?;
    if major(device["initial_support"].as_str().unwrap_or("99")) > target {
        bail!("SMBIOS {model} новее выбранной macOS");
    }
    let mut decisions = vec![
        format!("SMBIOS: {model}"),
        format!("Wi-Fi: {}", chosen.as_deref().unwrap_or("не выбран")),
    ];
    decisions.extend(disabled.iter().map(|n| format!("Отключить: {n}")));
    Ok(Plan {
        report_sha256: report.sha256.clone(),
        target,
        smbios: model,
        kexts,
        artifacts,
        disabled,
        needs_oclp: needs_oclp || (target == 25 && layout.is_some()),
        nvidia,
        audio_layout: layout,
        notices,
        hardware: report.hardware,
        decisions,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dependency_order() {
        let r = resolve(&BTreeSet::from(["VirtualSMC".into()]), 24).unwrap();
        assert_eq!(r[0].name, "Lilu");
        assert_eq!(r[1].name, "VirtualSMC");
    }
    #[test]
    fn gpu_kext_conflict_rejected() {
        assert!(
            resolve(
                &BTreeSet::from(["NootedRed".into(), "WhateverGreen".into()]),
                24
            )
            .is_err()
        );
    }
    #[test]
    fn all_exported_dependencies_exist() {
        for k in KEXTS.iter() {
            for d in &k.requires {
                assert!(KEXTS.iter().any(|k| &k.name == d), "{d}");
            }
        }
    }
}
