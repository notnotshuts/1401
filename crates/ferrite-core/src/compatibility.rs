use crate::{
    RULES_REVISION,
    report::{Report, text},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::LazyLock;

static GPU_RANGES: LazyLock<Value> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../data/gpu-ranges.json")).expect("bundled GPU ranges")
});
static PCI: LazyLock<Value> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../data/pci-rules.json")).expect("bundled PCI rules")
});
static NVIDIA: LazyLock<Value> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../data/nvidia-gsp.json")).expect("bundled NVIDIA table")
});

pub const VERSIONS: [(u8, &str); 7] = [
    (19, "Catalina 10.15"),
    (20, "Big Sur 11"),
    (21, "Monterey 12"),
    (22, "Ventura 13"),
    (23, "Sonoma 14"),
    (24, "Sequoia 15"),
    (25, "Tahoe 26"),
];
pub fn version_name(major: u8) -> &'static str {
    VERSIONS
        .iter()
        .find(|v| v.0 == major)
        .map(|v| v.1)
        .unwrap_or("Неизвестная версия")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Supported,
    Driver,
    Patched,
    Unsupported,
    Unknown,
}
impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Self::Supported => "Поддерживается",
            Self::Driver => "NVIDIA-драйвер",
            Self::Patched => "Нужен OCLP",
            Self::Unsupported => "Не поддерживается",
            Self::Unknown => "Не проверено",
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Range {
    pub min: (u8, u8, u8),
    pub max: (u8, u8, u8),
}
impl Range {
    pub fn major(min: u8, max: u8) -> Self {
        Self {
            min: (min, 0, 0),
            max: (max, 99, 99),
        }
    }
    pub fn contains(self, target: u8) -> bool {
        self.min <= (target, 99, 99) && self.max >= (target, 0, 0) && self.min <= self.max
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Device {
    pub section: String,
    pub name: String,
    pub id: String,
    pub status: Status,
    pub native: Option<Range>,
    pub patched: Option<Range>,
    pub reason: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Analysis {
    pub schema: u32,
    pub target: u8,
    pub report_sha256: String,
    pub rules_revision: String,
    pub devices: Vec<Device>,
    pub native_versions: Vec<u8>,
    pub patched_versions: Vec<u8>,
    pub recommended: Option<u8>,
    pub blockers: Vec<String>,
    pub warnings: Vec<String>,
    pub ok: bool,
}
fn in_table(table: &str, id: &str) -> bool {
    PCI[table]
        .as_array()
        .is_some_and(|a| a.iter().any(|v| v.as_str() == Some(id)))
}
fn index(table: &str, id: &str) -> Option<usize> {
    PCI[table]
        .as_array()?
        .iter()
        .position(|v| v.as_str() == Some(id))
}
fn features(report: &Report, feature: &str) -> bool {
    let raw = report.hardware["CPU"]["SIMD Features"]
        .to_string()
        .to_uppercase();
    let tokens: Vec<_> = raw
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '.')
        .collect();
    if feature == "SSE4" {
        tokens.contains(&"SSE4.1") || tokens.contains(&"SSE4.2")
    } else {
        tokens.contains(&feature)
    }
}
pub fn classify_gpu(id: &str) -> (&'static str, &'static str, &'static str) {
    let vendor = id.get(..4).unwrap_or("");
    let manufacturer = match vendor {
        "8086" => "Intel",
        "1002" => "AMD",
        "10DE" => "NVIDIA",
        _ => "Unknown",
    };
    if let Some(pid) = id.get(5..).and_then(|p| u64::from_str_radix(p, 16).ok())
        && let Some(rows) = GPU_RANGES[vendor].as_array()
    {
        for r in rows {
            if r["start"].as_u64().unwrap_or(65536) <= pid && pid <= r["end"].as_u64().unwrap_or(0)
            {
                return (
                    manufacturer,
                    r["codename"].as_str().unwrap_or("Unknown"),
                    r["device_type"].as_str().unwrap_or("Unknown"),
                );
            }
        }
    }
    (manufacturer, "Unknown", "Unknown")
}
fn has_disks(props: &Value) -> bool {
    match &props["Disk Drives"] {
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
        _ => false,
    }
}
fn known_nvidia(props: &Value, id: &str) -> bool {
    text(props, "Manufacturer") == "NVIDIA"
        && text(props, "Device Type") == "Discrete GPU"
        && id.starts_with("10DE-")
        && NVIDIA["ids"].get(&id[5..]).is_some()
}

fn gpu_range(
    report: &Report,
    props: &Value,
    id: &str,
) -> (Option<Range>, Option<Range>, bool, String) {
    let manufacturer = text(props, "Manufacturer");
    let declared = text(props, "Codename");
    let code = if declared.is_empty() || declared == "Unknown" {
        classify_gpu(id).1
    } else {
        declared
    };
    let pid = id.get(5..).unwrap_or("");
    if !id.is_ascii()
        || id.len() != 9
        || id.as_bytes()[4] != b'-'
        || !id[..4]
            .bytes()
            .chain(id[5..].bytes())
            .all(|b| b.is_ascii_hexdigit())
    {
        return (
            None,
            None,
            false,
            "Не указан корректный PCI ID видеокарты".into(),
        );
    }
    let mut range = Some(Range::major(17, 25));
    let mut patch_min = 20;
    let mut patch_max = 24;
    if known_nvidia(props, id) {
        return (
            Some(Range::major(24, 24)),
            None,
            true,
            format!(
                "Таблица NVIDIA GSP; только Sequoia. {}",
                if NVIDIA["tested"]
                    .as_array()
                    .is_some_and(|a| a.iter().any(|x| x.as_str() == Some(pid)))
                {
                    "Есть тест драйвера."
                } else {
                    "Карта в таблице, фактический запуск драйвера не подтверждён."
                }
            ),
        );
    }
    if manufacturer.contains("Intel") {
        if pid.len() != 4 {
            range = None;
        } else if (matches!(pid, "0042" | "0046")
            && report.text("Motherboard", "Platform") != "Desktop")
            || (pid.starts_with("01")
                && !matches!(pid.as_bytes()[2], b'5' | b'6')
                && !matches!(pid, "0102" | "0106" | "010A"))
        {
            range = Some(Range::major(17, 17));
        } else if pid.starts_with("01") && !matches!(pid, "0152" | "0156") {
            range = Some(Range::major(17, 20));
        } else if ["04", "0A", "0C", "0D", "0B", "16"]
            .iter()
            .any(|p| pid.starts_with(p))
        {
            range = Some(Range::major(17, 21));
        } else if ["09", "19", "59", "3E", "87", "9B"]
            .iter()
            .any(|p| pid.starts_with(p))
            && ![
                "3E90", "3E93", "3E99", "3E9C", "3EA1", "3EA4", "9B21", "9BA0", "9BA2", "9BA4",
                "9BA5", "9BA8", "9BAA", "9BAB", "9BAC",
            ]
            .contains(&pid)
        {
        } else if pid.starts_with("8A") {
            range = Some(Range {
                min: (19, 4, 0),
                max: (25, 99, 99),
            });
        } else {
            range = None;
        }
        if ["Celeron", "Pentium"]
            .iter()
            .any(|n| report.text("CPU", "Processor Name").contains(n))
        {
            range = None;
        }
    } else if manufacturer.contains("AMD") {
        let mut min = (17, 0, 0);
        let mut max = 25;
        if code.contains("Navi 2") {
            if !features(report, "AVX2") {
                max = 21;
                patch_max = 21;
            } else if matches!(code, "Navi 23" | "Navi 22") {
                min = (21, 2, 0);
            } else if code.contains("Navi 21") {
                min = (20, 5, 0);
            } else {
                range = None;
            }
        } else if code.contains("Navi 1") {
            min = (19, 0, 0);
            if !features(report, "AVX2") {
                max = 21;
                patch_min = 22;
            }
        } else if code == "Vega 20" {
            min = (18, 6, 0);
            if !features(report, "AVX2") {
                max = 21;
                patch_min = 22;
            }
        } else if ["Vega 10", "Polaris 22", "Polaris 20", "Baffin", "Ellesmere"].contains(&code)
            || matches!(pid, "6995" | "699F")
        {
            if !features(report, "AVX2") {
                max = 21;
                patch_min = 22;
            }
        } else if [
            "Hainan",
            "Oland",
            "Cape Verde",
            "Pitcairn",
            "Tahiti",
            "Bonaire",
            "Hawaii",
            "Tonga",
            "Fiji",
        ]
        .iter()
        .any(|c| code.contains(c))
        {
            max = 21;
        } else if ["15D8", "15DD", "15E7", "1636", "1638", "164C"].contains(&pid) {
            min = (19, 0, 0);
        } else {
            range = None;
        }
        if range.is_some() {
            range = Some(Range {
                min,
                max: (max, 99, 99),
            });
        }
    } else if manufacturer.contains("NVIDIA") {
        range = if code.contains("Kepler") {
            Some(Range::major(17, 20))
        } else if ["Pascal", "Maxwell", "Fermi", "Tesla"].contains(&code) {
            Some(Range::major(17, 17))
        } else {
            None
        };
    } else {
        return (
            None,
            None,
            false,
            "Неизвестный производитель или архитектура GPU".into(),
        );
    }
    let patched = range.and_then(|r| {
        (r.max.0 < patch_max).then(|| Range::major(patch_min.max(r.max.0 + 1), patch_max))
    });
    (
        range,
        patched,
        false,
        "Правила GPU закреплённой OpCore-Simplify; работа экрана требует отдельной проверки".into(),
    )
}
pub fn analyze(report: &Report, target: u8) -> Analysis {
    let mut a = Analysis {
        schema: 1,
        target,
        report_sha256: report.sha256.clone(),
        rules_revision: RULES_REVISION.into(),
        devices: vec![],
        native_versions: vec![],
        patched_versions: vec![],
        recommended: None,
        blockers: vec![],
        warnings: vec![],
        ok: false,
    };
    if !VERSIONS.iter().any(|v| v.0 == target) {
        a.blockers.push("Выберите Darwin major 19–25".into());
    }
    for key in ["Scan Errors", "Scan Notes"] {
        if let Some(rows) = report.hardware[key].as_array() {
            for r in rows {
                if let Some(s) = r.as_str() {
                    if key == "Scan Errors" {
                        a.blockers.push(s.into());
                    } else {
                        a.warnings.push(s.into());
                    }
                }
            }
        }
    }
    let cpu_name = report.text("CPU", "Processor Name");
    let cpu = if !features(report, "SSE4") {
        None
    } else if !features(report, "SSE4.2") {
        Some(Range::major(18, 21))
    } else {
        Some(Range::major(17, 25))
    };
    if cpu_name.is_empty() {
        a.blockers.push("В отчёте нет названия CPU".into());
    }
    if cpu.is_none() {
        a.blockers.push("Не подтверждены инструкции SSE4.x".into());
    }
    a.devices.push(Device {
        section: "CPU".into(),
        name: cpu_name.into(),
        id: String::new(),
        status: if cpu.is_some_and(|r| r.contains(target)) {
            Status::Supported
        } else {
            Status::Unsupported
        },
        native: cpu,
        patched: None,
        reason: "SSE4.1: до Monterey; SSE4.2: диапазон таблиц 1401".into(),
    });
    for section in [
        "GPU",
        "Network",
        "Storage Controllers",
        "Sound",
        "Bluetooth",
        "SD Controller",
        "Biometric",
    ] {
        for (name, p) in report.rows(section) {
            let id = text(p, "Device ID").to_uppercase();
            let mut native = Some(Range::major(17, 25));
            let mut patched = None;
            let mut driver = false;
            let mut unknown = false;
            let reason;
            match section {
                "GPU" => {
                    (native, patched, driver, reason) = gpu_range(report, p, &id);
                }
                "Network" => {
                    if let Some(i) = index("BroadcomWiFiIDs", &id) {
                        if i == 13 || (18..21).contains(&i) {
                            native = Some(Range::major(17, 22));
                            patched = Some(Range::major(23, 25));
                        } else if i < 12 {
                            native = Some(Range::major(17, 17));
                            patched = Some(Range::major(20, 25));
                        }
                    } else if let Some(i) = index("AtherosWiFiIDs", &id) {
                        native = Some(Range::major(17, if i < 8 { 17 } else { 20 }));
                    } else if in_table("IntelI22XIDs", &id) {
                        native = Some(Range::major(19, 25));
                    } else if in_table("AquantiaAqtionIDs", &id) {
                        native = Some(Range::major(21, 25));
                    }
                    if in_table("WirelessCardIDs", &id)
                        && !in_table("IntelWiFiIDs", &id)
                        && !index("AtherosWiFiIDs", &id).is_some_and(|i| i >= 8)
                        && !in_table("rtw88WiFiIDs", &id)
                        && patched.is_none()
                    {
                        patched = Some(Range::major(20, 25));
                    }
                    if !(in_table("EthernetIDs", &id)
                        || in_table("WirelessUSBIDs", &id)
                        || in_table("WirelessCardIDs", &id))
                    {
                        native = None;
                        unknown = !text(p, "Bus Type").starts_with("PCI");
                    }
                    reason = "PCI/USB ID в закреплённой таблице сетевых устройств".into();
                }
                "Storage Controllers" => {
                    if text(p, "Bus Type") != "PCI" || !has_disks(p) {
                        native = None;
                        unknown = true;
                    }
                    if in_table("IntelVMDIDs", &id) {
                        native = None;
                        a.blockers.push(format!(
                            "{name}: выключите Intel VMD и повторите сканирование"
                        ));
                    }
                    if PCI["UnsupportedNVMeSSDIDs"][&id]
                        .as_array()
                        .is_some_and(|ids| {
                            ids.iter()
                                .any(|s| s.as_str() == Some(text(p, "Subsystem ID")))
                        })
                    {
                        native = None;
                    }
                    if id == "1C5C-174A" && !unknown && native.is_some() {
                        native = Some(Range::major(20, 25));
                    }
                    reason = "PCI-контроллер с диском; проверка VMD и NVMe subsystem ID".into();
                }
                "Sound" => {
                    if !(text(p, "Bus Type").contains("USB")
                        || id.starts_with("1002")
                        || (id.starts_with("8086") && !in_table("IntelSSTIDs", &id))
                        || in_table("AudioCodecIDs", &id))
                    {
                        native = None;
                    }
                    reason = "Таблица аудиокодеков; PCI audio не подтверждает работу всех выходов"
                        .into();
                }
                "Bluetooth" => {
                    if in_table("AtherosBluetoothIDs", &id) {
                        native = Some(Range::major(17, 20));
                    } else if !in_table("BluetoothIDs", &id) {
                        native = None;
                    }
                    reason = "Таблица Bluetooth ID".into();
                }
                "SD Controller" => {
                    if let Some(i) = index("RealtekCardReaderIDs", &id) {
                        if i < 5 {
                            native = Some(Range::major(17, 23));
                        }
                    } else {
                        native = None;
                    }
                    reason = "Таблица картридеров Realtek".into();
                }
                _ => {
                    native = None;
                    reason = "Биометрия Hackintosh не поддерживается без Apple T2".into();
                }
            }
            let status = if native.is_some_and(|r| r.contains(target)) {
                if driver {
                    Status::Driver
                } else {
                    Status::Supported
                }
            } else if patched.is_some_and(|r| r.contains(target)) {
                Status::Patched
            } else if unknown {
                Status::Unknown
            } else {
                Status::Unsupported
            };
            a.devices.push(Device {
                section: section.into(),
                name: name.into(),
                id,
                status,
                native,
                patched,
                reason,
            });
        }
    }
    if !a.devices.iter().any(|d| d.section == "GPU") {
        a.blockers.push("Нет сведений о GPU".into());
    }
    if !a
        .devices
        .iter()
        .any(|d| d.section == "Storage Controllers" && d.native.is_some())
    {
        a.blockers
            .push("Нет проверенного PCI-контроллера с подключённым диском".into());
    }
    // Internal display routing cannot be inferred from NVIDIA firmware support.
    for (name, monitor) in report.rows("Monitor") {
        if ["Internal", "eDP", "LVDS"].contains(&text(monitor, "Connector Type")) {
            let gpu = text(monitor, "Connected GPU");
            if gpu.is_empty() {
                a.warnings.push(format!(
                    "{name}: не определена разводка встроенного экрана; проверьте MUX"
                ));
            } else if !a.devices.iter().any(|d| {
                d.section == "GPU" && d.name == gpu && d.native.is_some_and(|r| r.contains(target))
            }) {
                a.warnings.push(format!("{name}: экран подключён к {gpu}, который не поддерживается выбранной macOS. NVIDIA не перенаправляет экран; нужен MUX или подходящий внешний выход."));
            }
        }
    }
    if a.blockers.is_empty() {
        for (v, _) in VERSIONS {
            let cpu_ok = cpu.is_some_and(|r| r.contains(v));
            let storage = a.devices.iter().any(|d| {
                d.section == "Storage Controllers" && d.native.is_some_and(|r| r.contains(v))
            });
            let gpu = a
                .devices
                .iter()
                .any(|d| d.section == "GPU" && d.native.is_some_and(|r| r.contains(v)));
            let gpu_patch = a
                .devices
                .iter()
                .any(|d| d.section == "GPU" && d.patched.is_some_and(|r| r.contains(v)));
            if cpu_ok && storage {
                if gpu {
                    a.native_versions.push(v);
                } else if gpu_patch {
                    a.patched_versions.push(v);
                }
            }
        }
    }
    a.recommended = a
        .native_versions
        .last()
        .or(a.patched_versions.last())
        .copied();
    a.ok = a.blockers.is_empty()
        && (a.native_versions.contains(&target) || a.patched_versions.contains(&target));
    if !a.ok && a.blockers.is_empty() {
        a.blockers.push(format!(
            "{} вне проверенного диапазона CPU/GPU/накопителя",
            version_name(target)
        ));
    }
    a.warnings.push("Оценка по таблицам не гарантирует загрузку macOS. ACPI и разводка экрана проверяются отдельно.".into());
    a
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn report(gpu: Value) -> Report {
        Report::from_bytes(&serde_json::to_vec(&json!({"CPU":{"Processor Name":"Intel Core i7","SIMD Features":"SSE4.1 SSE4.2 AVX2"},"GPU":{"GPU":gpu},"Storage Controllers":{"NVMe":{"Bus Type":"PCI","Device ID":"144D-A808","Disk Drives":{"SSD":{}}}}})).unwrap()).unwrap()
    }
    #[test]
    fn array_disk_report_is_supported() {
        let mut r =
            report(json!({"Manufacturer":"AMD","Codename":"Navi 23","Device ID":"1002-73FF"}));
        r.hardware["Storage Controllers"]["NVMe"]["Disk Drives"] = json!(["SSD"]);
        assert!(analyze(&r, 24).ok);
    }
    #[test]
    fn gpu_identification_respects_order() {
        assert_eq!(classify_gpu("1002-73FF").1, "Navi 23");
        assert_eq!(classify_gpu("10DE-1140").1, "Fermi");
        assert_eq!(classify_gpu("8086-9A49").1, "Tiger Lake");
    }
    #[test]
    fn forged_sse_flag_is_not_evidence() {
        let mut r =
            report(json!({"Manufacturer":"AMD","Codename":"Navi 23","Device ID":"1002-73FF"}));
        r.hardware["CPU"]["SIMD Features"] = json!("SSE4.2FAKE");
        assert!(!analyze(&r, 24).ok);
    }
    #[test]
    fn vmd_blocks_even_with_another_storage_controller() {
        let mut r =
            report(json!({"Manufacturer":"AMD","Codename":"Navi 23","Device ID":"1002-73FF"}));
        let id = PCI["IntelVMDIDs"].as_array().unwrap()[0].clone();
        r.hardware["Storage Controllers"]["VMD"] =
            json!({"Bus Type":"PCI","Device ID":id,"Disk Drives":["SSD"]});
        assert!(!analyze(&r, 24).ok);
    }
    #[test]
    fn unknown_nvidia_never_passes() {
        let a = analyze(
            &report(
                json!({"Manufacturer":"NVIDIA","Device Type":"Discrete GPU","Device ID":"10DE-FFFF"}),
            ),
            24,
        );
        assert!(!a.ok);
    }
    #[test]
    fn firmware_table_is_sequoia_only() {
        let r = report(
            json!({"Manufacturer":"NVIDIA","Device Type":"Discrete GPU","Device ID":"10DE-1E04"}),
        );
        assert!(analyze(&r, 24).ok);
        assert!(!analyze(&r, 25).ok);
        assert_eq!(analyze(&r, 24).devices[1].status, Status::Driver);
    }
    #[test]
    fn wifi_patch_cannot_rescue_unknown_gpu() {
        let mut r =
            report(json!({"Manufacturer":"AMD","Codename":"Unknown","Device ID":"1002-FFFF"}));
        r.hardware["Network"] = json!({"wifi":{"Device ID":"14E4-43A0","Bus Type":"PCI"}});
        assert!(analyze(&r, 24).native_versions.is_empty());
    }
    #[test]
    fn supported_gpu_needs_a_disk() {
        let mut r =
            report(json!({"Manufacturer":"AMD","Codename":"Navi 23","Device ID":"1002-73FF"}));
        r.hardware["Storage Controllers"] = json!({});
        assert!(!analyze(&r, 24).ok);
    }
    #[test]
    fn patch_limits_are_preserved() {
        let r = report(json!({"Manufacturer":"Intel","Device ID":"8086-8A52"}));
        let a = analyze(&r, 19);
        assert_eq!(a.devices[1].native.unwrap().min, (19, 4, 0));
    }
}
