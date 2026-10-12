//! OpenCore configuration built from the pinned release's own Sample.plist.
use crate::{
    planner::{self, Options, Plan},
    report::{Report, text},
};
use anyhow::{Context, Result, bail};
use plist::{Dictionary, Value};
use rand::RngCore;
use std::path::Path;
pub const BOOT: &str = "7C436110-AB2A-4BBB-A880-FE41995C9F82";
pub fn bytes(hex: &str) -> Result<Vec<u8>> {
    if !hex.is_ascii() || !hex.len().is_multiple_of(2) {
        bail!("Invalid hex data");
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).context("Invalid hex byte"))
        .collect()
}
pub fn dict() -> Value {
    Value::Dictionary(Dictionary::new())
}
pub fn set(root: &mut Value, path: &[&str], value: Value) -> Result<()> {
    let (key, rest) = path.split_first().context("Empty plist path")?;
    let map = root
        .as_dictionary_mut()
        .context("Plist path must be dictionary")?;
    if rest.is_empty() {
        map.insert((*key).into(), value);
        return Ok(());
    }
    if !map.contains_key(key) {
        map.insert((*key).into(), dict());
    }
    set(map.get_mut(key).context("Plist key")?, rest, value)
}
pub fn get<'a>(root: &'a Value, path: &[&str]) -> Option<&'a Value> {
    if path.is_empty() {
        return Some(root);
    }
    get(root.as_dictionary()?.get(path[0])?, &path[1..])
}
pub fn json(value: serde_json::Value) -> Result<Value> {
    Ok(match value {
        serde_json::Value::Null => bail!("Null cannot be encoded as plist"),
        serde_json::Value::Bool(b) => Value::Boolean(b),
        serde_json::Value::String(s) => Value::String(s),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::Integer(i.into())
            } else {
                Value::Integer(n.as_u64().context("Plist integer")?.into())
            }
        }
        serde_json::Value::Array(a) => {
            Value::Array(a.into_iter().map(json).collect::<Result<_>>()?)
        }
        serde_json::Value::Object(m) => Value::Dictionary(
            m.into_iter()
                .map(|(k, v)| Ok((k, json(v)?)))
                .collect::<Result<_>>()?,
        ),
    })
}
fn integer(n: i64) -> Value {
    Value::Integer(n.into())
}
fn boolean(cfg: &mut Value, path: &[&str], v: bool) -> Result<()> {
    set(cfg, path, Value::Boolean(v))
}
fn add_data(props: &mut Dictionary, key: &str, hex: &str) -> Result<()> {
    props.insert(key.into(), Value::Data(bytes(hex)?));
    Ok(())
}
fn model_id(id: &str) -> Option<u16> {
    id.get(5..).and_then(|s| u16::from_str_radix(s, 16).ok())
}
pub fn igpu(r: &Report, name: &str, g: &serde_json::Value, target: u8) -> Result<Dictionary> {
    let id = text(g, "Device ID");
    let pid = id.get(5..).unwrap_or("");
    let platform = if r.text("Motherboard", "Name").contains("NUC") {
        "NUC"
    } else {
        r.text("Motherboard", "Platform")
    };
    let laptop = platform == "Laptop";
    let attached = r
        .rows("Monitor")
        .iter()
        .any(|(_, m)| text(m, "Connected GPU") == name && text(m, "Connector Type") != "VGA");
    let highres = r.rows("Monitor").iter().any(|(_, m)| {
        text(m, "Resolution")
            .split_once('x')
            .and_then(|(w, h)| Some((w.parse::<u32>().ok()?, h.parse::<u32>().ok()?)))
            .is_some_and(|(w, h)| w >= 3840 && h >= 2160)
    });
    let mut p = Dictionary::new();
    let (frame, spoof) = if pid.starts_with("01") {
        if pid
            .as_bytes()
            .get(2)
            .is_some_and(|c| matches!(c, b'5' | b'6'))
        {
            (
                if laptop {
                    "03006601"
                } else if platform == "NUC" {
                    "0B006601"
                } else if attached {
                    "0A006601"
                } else {
                    "07006201"
                },
                Some("62010000"),
            )
        } else {
            (
                if laptop {
                    "00000100"
                } else if attached {
                    "10000300"
                } else {
                    "00000500"
                },
                Some("26010000"),
            )
        }
    } else if ["04", "0A", "0C", "0D"].iter().any(|x| pid.starts_with(x)) {
        (
            if laptop {
                "0600260A"
            } else if !attached && platform == "Desktop" {
                "04001204"
            } else {
                "0300220D"
            },
            Some("12040000"),
        )
    } else if pid.starts_with("0B") || pid.starts_with("16") {
        (
            if laptop {
                "06002616"
            } else if platform == "NUC" {
                "02001616"
            } else {
                "07002216"
            },
            Some("26160000"),
        )
    } else if (pid.starts_with("09") || pid.starts_with("19")) && target < 22 {
        (
            if laptop {
                "00001619"
            } else if platform == "NUC" {
                "02001619"
            } else if attached {
                "00001219"
            } else {
                "01001219"
            },
            Some("1B190000"),
        )
    } else if ["09", "19", "59", "87C0"]
        .iter()
        .any(|x| pid.starts_with(x))
    {
        (
            if laptop {
                if pid == "5917" || pid == "87C0" {
                    "00001659"
                } else {
                    "00001B59"
                }
            } else if platform == "NUC" {
                "00001E59"
            } else if attached {
                "00001259"
            } else {
                "03001259"
            },
            Some("16590000"),
        )
    } else if ["3E", "87", "9B"].iter().any(|x| pid.starts_with(x)) {
        (
            if laptop {
                "0900A53E"
            } else if platform == "NUC" {
                "07009B3E"
            } else if attached {
                "00009B3E"
            } else if pid.starts_with("9B") {
                "0300C89B"
            } else {
                "0300913E"
            },
            Some("9B3E0000"),
        )
    } else if pid.starts_with("8A") {
        ("0200518A", Some("528A0000"))
    } else {
        return Ok(p);
    };
    add_data(
        &mut p,
        if pid.starts_with("01")
            && !pid
                .as_bytes()
                .get(2)
                .is_some_and(|c| matches!(c, b'5' | b'6'))
        {
            "AAPL,snb-platform-id"
        } else {
            "AAPL,ig-platform-id"
        },
        frame,
    )?;
    // Preserve native IDs; spoof only when the selected framebuffer belongs to a different device family.
    if let Some(spoof) = spoof {
        let native = [
            "0D26", "0A26", "0A2E", "0D22", "0412", "1606", "160E", "161E", "1626", "1622", "1612",
            "162B", "1916", "191E", "1926", "1927", "1912", "1932", "1902", "1917", "193B", "191B",
            "5912", "5916", "591B", "591C", "591E", "5926", "5927", "5923", "87C0", "3E9B", "3EA5",
            "3EA6", "3E92", "3E91", "3E98", "9BC8", "9BC5", "9BC4", "8A70", "8A71", "8A51", "8A5C",
            "8A5D", "8A52", "8A53", "8A5A", "8A5B", "0152", "0156", "0162", "0166", "0106", "0116",
            "0126", "0102",
        ];
        if !native.contains(&pid) {
            add_data(&mut p, "device-id", spoof)?;
        }
    }
    if attached || laptop {
        add_data(&mut p, "framebuffer-patch-enable", "01000000")?;
        if !highres {
            add_data(&mut p, "framebuffer-stolenmem", "00003001")?;
            if laptop || pid.starts_with("16") || pid.starts_with("59") {
                add_data(&mut p, "framebuffer-fbmem", "00009000")?;
            }
        }
    }
    if pid.starts_with("8A") {
        for k in [
            "enable-dbuf-early-optimizer",
            "enable-dvmt-calc-fix",
            "enable-cdclk-frequency-fix",
        ] {
            add_data(&mut p, k, "01000000")?;
        }
    }
    if laptop && highres {
        add_data(&mut p, "enable-max-pixel-clock-override", "01000000")?;
    }
    Ok(p)
}
pub fn generate(
    sample: &Path,
    plan: &Plan,
    options: &Options,
    serial: &str,
    mlb: &str,
) -> Result<Value> {
    if !(serial.len() >= 11
        && serial.len() <= 12
        && serial.bytes().all(|b| b.is_ascii_alphanumeric())
        && mlb.len() == 17
        && mlb.bytes().all(|b| b.is_ascii_alphanumeric()))
    {
        bail!("macserial вернул некорректные значения");
    }
    let mut cfg = Value::from_file(sample)?;
    let map = cfg.as_dictionary_mut().context("Sample.plist")?;
    map.retain(|k, _| !k.starts_with('#'));
    let r = Report {
        hardware: plan.hardware.clone(),
        sha256: plan.report_sha256.clone(),
    };
    let amd = r.text("CPU", "Manufacturer").contains("AMD");
    let code = r.text("CPU", "Codename");
    let board = r.text("Motherboard", "Name").to_uppercase();
    let chipset = r.text("Motherboard", "Chipset");
    let intel_idx = planner::platform_index("IntelCPUGenerations", code);
    let chip_idx = planner::platform_index("IntelChipsets", chipset);
    let modern = chip_idx.is_some_and(|i| i >= 112) || intel_idx.is_some_and(|i| i < 20);
    let has = |name: &str| plan.kexts.iter().any(|k| k.name == name);
    for (section, key) in [
        ("ACPI", "Add"),
        ("ACPI", "Delete"),
        ("ACPI", "Patch"),
        ("Booter", "Patch"),
        ("Booter", "MmioWhitelist"),
        ("Kernel", "Add"),
        ("Kernel", "Block"),
        ("Kernel", "Force"),
        ("Kernel", "Patch"),
        ("Misc", "BlessOverride"),
        ("Misc", "Entries"),
        ("Misc", "Tools"),
        ("UEFI", "Drivers"),
        ("UEFI", "ReservedMemory"),
    ] {
        set(&mut cfg, &[section, key], Value::Array(vec![]))?;
    }
    let write_unprotector = !amd && !modern;
    for (key, v) in [
        ("AvoidRuntimeDefrag", true),
        ("EnableSafeModeSlide", true),
        ("EnableWriteUnprotector", write_unprotector),
        ("RebuildAppleMemoryMap", !write_unprotector),
        ("ProtectUefiServices", modern),
        ("ProvideCustomSlide", true),
        ("SyncRuntimePermissions", amd || modern),
        (
            "SetupVirtualMap",
            !amd && !chip_idx.is_some_and(|i| (90..100).contains(&i)),
        ),
        (
            "DevirtualiseMmio",
            if amd {
                matches!(chipset, "TRX40" | "TRX50")
            } else {
                modern
            },
        ),
        ("AllowRelocationBlock", true),
    ] {
        boolean(&mut cfg, &["Booter", "Quirks", key], v)?;
    }
    set(
        &mut cfg,
        &["Booter", "Quirks", "ResizeAppleGpuBars"],
        integer(
            if plan.nvidia
                || r.rows("GPU")
                    .iter()
                    .any(|(_, g)| text(g, "Resizable BAR") == "Enabled")
            {
                0
            } else {
                -1
            },
        ),
    )?;
    let legacy_pm = intel_idx.is_some_and(|i| i >= 62);
    for (key, v) in [
        ("AppleCpuPmCfgLock", legacy_pm),
        ("AppleXcpmCfgLock", !amd && !legacy_pm),
        (
            "AppleXcpmExtraMsrs",
            planner::is_hedt(&r) && intel_idx.is_some_and(|i| i >= 50),
        ),
        ("AppleXcpmForceBoost", intel_idx.is_some_and(|i| i < 4)),
        ("CustomSMBIOSGuid", true),
        ("DisableIoMapper", !amd),
        (
            "DisableRtcChecksum",
            board.contains("ASUS") || board.contains("HP "),
        ),
        (
            "ForceAquantiaEthernet",
            r.rows("Network")
                .iter()
                .any(|(_, n)| planner::table("AquantiaAqtionIDs", text(n, "Device ID"))),
        ),
        ("LapicKernelPanic", board.contains("HP ")),
        ("PanicNoKextDump", true),
        ("PowerTimeoutKernelPanic", true),
        (
            "ProvideCurrentCpuInfo",
            amd || intel_idx.is_some_and(|i| (4..20).contains(&i)),
        ),
    ] {
        boolean(&mut cfg, &["Kernel", "Quirks", key], v)?;
    }
    boolean(
        &mut cfg,
        &["Kernel", "Emulate", "DummyPowerManagement"],
        amd,
    )?;
    let spoof = if code.contains("Tiger Lake") {
        Some("E5060700")
    } else if !code.contains("Ice Lake") && intel_idx.is_some_and(|i| i < 30) {
        Some("55060A00")
    } else if planner::is_hedt(&r) && code.contains("Haswell") {
        Some("C3060300")
    } else if planner::is_hedt(&r) && code.contains("Broadwell") {
        Some("D4060300")
    } else {
        None
    };
    set(
        &mut cfg,
        &["Kernel", "Emulate", "Cpuid1Data"],
        Value::Data(if let Some(hex) = spoof {
            bytes(&format!("{hex}000000000000000000000000"))?
        } else {
            vec![]
        }),
    )?;
    set(
        &mut cfg,
        &["Kernel", "Emulate", "Cpuid1Mask"],
        Value::Data(if spoof.is_some() {
            bytes("FFFFFFFF000000000000000000000000")?
        } else {
            vec![]
        }),
    )?;
    for (key, v) in [
        ("AppleDebug", true),
        ("ApplePanic", true),
        ("DisableWatchDog", true),
    ] {
        boolean(&mut cfg, &["Misc", "Debug", key], v)?;
    }
    set(&mut cfg, &["Misc", "Debug", "Target"], integer(65))?;
    set(
        &mut cfg,
        &["Misc", "Boot", "PickerMode"],
        Value::String("Builtin".into()),
    )?;
    set(&mut cfg, &["Misc", "Boot", "Timeout"], integer(5))?;
    {
        let (key, v) = ("AllowSetDefault", true);
        boolean(&mut cfg, &["Misc", "Security", key], v)?;
    }
    set(&mut cfg, &["Misc", "Security", "ScanPolicy"], integer(0))?;
    set(
        &mut cfg,
        &["Misc", "Security", "ExposeSensitiveData"],
        integer(7),
    )?;
    set(
        &mut cfg,
        &["Misc", "Security", "Vault"],
        Value::String("Optional".into()),
    )?;
    set(
        &mut cfg,
        &["Misc", "Security", "SecureBootModel"],
        Value::String(
            if !plan.needs_oclp && (20..23).contains(&plan.target) {
                "Default"
            } else {
                "Disabled"
            }
            .into(),
        ),
    )?;
    set(&mut cfg, &["NVRAM", "Add"], dict())?;
    set(&mut cfg, &["NVRAM", "Delete"], dict())?;
    let mut args = if options.verbose {
        vec!["-v".to_string(), "debug=0x100".into(), "keepsyms=1".into()]
    } else {
        vec![]
    };
    if has("WhateverGreen")
        && r.rows("GPU").iter().any(|(name, g)| {
            text(g, "Codename").contains("Navi") && !plan.disabled.iter().any(|d| d == name)
        })
    {
        args.push("agdpmod=pikera".into());
    }
    if let Some(layout) = plan.audio_layout {
        args.push(format!("alcid={layout}"));
    }
    if has("CpuTopologyRebuild") {
        args.push("ctrsmt=full".into());
    }
    if has("VoodooI2C") {
        args.push("-vi2c-force-polling".into());
    }
    if has("IntelBluetoothFirmware") && plan.target == 25 {
        args.push("-ibtcompatbeta".into());
    }
    if plan.needs_oclp && plan.target == 25 {
        args.push("-amfipassbeta".into());
    }
    if plan.nvidia {
        args.extend(
            [
                "nvfb=1",
                "nvaccel=1",
                "nvfbheads=4",
                "-nvkmsnosmooth",
                "amfi_get_out_of_my_way=0x1",
                "amfi=0x80",
            ]
            .map(str::to_string),
        );
    }
    set(
        &mut cfg,
        &["NVRAM", "Add", BOOT, "boot-args"],
        Value::String(args.join(" ")),
    )?;
    set(
        &mut cfg,
        &["NVRAM", "Add", BOOT, "csr-active-config"],
        Value::Data(if plan.nvidia {
            bytes("430A0000")?
        } else if plan.needs_oclp {
            bytes("030A0000")?
        } else {
            vec![0; 4]
        }),
    )?;
    set(
        &mut cfg,
        &["NVRAM", "Add", BOOT, "prev-lang:kbd"],
        Value::Data(vec![]),
    )?;
    set(
        &mut cfg,
        &["NVRAM", "Delete", BOOT],
        Value::Array(vec![Value::String("csr-active-config".into())]),
    )?;
    if has("BlueToolFixup") {
        for (key, data) in [
            ("bluetoothExternalDongleFailed", "00"),
            (
                "bluetoothInternalControllerInfo",
                "0000000000000000000000000000",
            ),
        ] {
            set(
                &mut cfg,
                &["NVRAM", "Add", BOOT, key],
                Value::Data(bytes(data)?),
            )?;
        }
    }
    let mut uuid = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut uuid);
    uuid[6] = (uuid[6] & 15) | 0x40;
    uuid[8] = (uuid[8] & 63) | 0x80;
    let encoded = uuid.iter().map(|b| format!("{b:02X}")).collect::<String>();
    let uuid = format!(
        "{}-{}-{}-{}-{}",
        &encoded[..8],
        &encoded[8..12],
        &encoded[12..16],
        &encoded[16..20],
        &encoded[20..]
    );
    let mut rom = [0u8; 6];
    rand::rngs::OsRng.fill_bytes(&mut rom);
    rom[0] = (rom[0] & 0xfe) | 2;
    set(
        &mut cfg,
        &["PlatformInfo", "Generic", "SystemProductName"],
        Value::String(plan.smbios.clone()),
    )?;
    set(
        &mut cfg,
        &["PlatformInfo", "Generic", "SystemSerialNumber"],
        Value::String(serial.into()),
    )?;
    set(
        &mut cfg,
        &["PlatformInfo", "Generic", "MLB"],
        Value::String(mlb.into()),
    )?;
    set(
        &mut cfg,
        &["PlatformInfo", "Generic", "SystemUUID"],
        Value::String(uuid),
    )?;
    set(
        &mut cfg,
        &["PlatformInfo", "Generic", "ROM"],
        Value::Data(rom.to_vec()),
    )?;
    set(
        &mut cfg,
        &["PlatformInfo", "UpdateSMBIOSMode"],
        Value::String("Custom".into()),
    )?;
    {
        let (key, v) = ("KeySupport", true);
        boolean(&mut cfg, &["UEFI", "Input", key], v)?;
    }
    for (key, v) in [
        (
            "EnableVectorAcceleration",
            intel_idx.is_none_or(|i| i >= 20),
        ),
        ("IgnoreInvalidFlexRatio", intel_idx.is_some_and(|i| i >= 50)),
        ("ReleaseUsbOwnership", true),
        ("UnblockFsConnect", board.contains("HP ")),
    ] {
        boolean(&mut cfg, &["UEFI", "Quirks", key], v)?;
    }
    set(&mut cfg, &["UEFI", "Quirks", "ResizeGpuBars"], integer(-1))?;
    set(&mut cfg, &["UEFI", "APFS", "MinDate"], integer(-1))?;
    set(&mut cfg, &["UEFI", "APFS", "MinVersion"], integer(-1))?;
    let mut props = Dictionary::new();
    for (name, g) in r.rows("GPU") {
        let path = text(g, "PCI Path");
        if path.is_empty() {
            continue;
        }
        if plan.disabled.contains(&name.to_string()) {
            let mut disabled = Dictionary::new();
            disabled.insert("disable-gpu".into(), Value::Boolean(true));
            props.insert(path.into(), Value::Dictionary(disabled));
            continue;
        }
        let mut p = if text(g, "Manufacturer") == "Intel"
            && text(g, "Device Type") == "Integrated GPU"
            && has("WhateverGreen")
        {
            igpu(&r, name, g, plan.target)?
        } else {
            Dictionary::new()
        };
        if plan.nvidia
            && text(g, "Manufacturer") == "NVIDIA"
            && r.text("Motherboard", "Platform") == "Laptop"
        {
            p.insert("acpi-wake-type".into(), integer(1));
        }
        if !p.is_empty() {
            props.insert(path.into(), Value::Dictionary(p));
        }
    }
    for section in ["Network", "Storage Controllers"] {
        for (name, d) in r.rows(section) {
            if plan.disabled.contains(&name.to_string()) {
                continue;
            }
            let path = text(d, "PCI Path");
            if path.is_empty() {
                continue;
            }
            let mut p = Dictionary::new();
            if text(d, "ACPI Path").is_empty() {
                p.insert("built-in".into(), Value::Data(vec![1]));
            }
            if section == "Network" && planner::table("BroadcomWiFiIDs", text(d, "Device ID")) {
                p.insert("IOName".into(), Value::String("pci14e4,43a0".into()));
            }
            if !p.is_empty() {
                props.insert(path.into(), Value::Dictionary(p));
            }
        }
    }
    set(
        &mut cfg,
        &["DeviceProperties", "Add"],
        Value::Dictionary(props),
    )?;
    set(&mut cfg, &["DeviceProperties", "Delete"], dict())?;
    if has("IOSkywalkFamily") {
        set(
            &mut cfg,
            &["Kernel", "Block"],
            Value::Array(vec![json(
                serde_json::json!({"Arch":"x86_64","Comment":"Allow IOSkywalk Downgrade","Enabled":true,"Identifier":"com.apple.iokit.IOSkywalkFamily","MaxKernel":"","MinKernel":"23.0.0","Strategy":"Exclude"}),
            )?]),
        )?;
    }
    if plan.target == 25
        && r.rows("GPU")
            .iter()
            .any(|(_, g)| text(g, "Manufacturer") == "AMD" && !has("WhateverGreen"))
    {
        let mut patch = match json(
            serde_json::json!({"Arch":"x86_64","Base":"","Comment":"Ferrite AGDP board-id to board-ix","Count":0,"Enabled":true,"Identifier":"com.apple.driver.AppleGraphicsDevicePolicy","Limit":0,"MaxKernel":"","MinKernel":"25.0.0","Skip":0}),
        )? {
            Value::Dictionary(d) => d,
            _ => unreachable!(),
        };
        for (k, v) in [
            ("Find", b"board-id".to_vec()),
            ("Replace", b"board-ix".to_vec()),
            ("Mask", vec![]),
            ("ReplaceMask", vec![]),
        ] {
            patch.insert(k.into(), Value::Data(v));
        }
        set(
            &mut cfg,
            &["Kernel", "Patch"],
            Value::Array(vec![Value::Dictionary(patch)]),
        )?;
    }
    let _ = model_id;
    Ok(cfg)
}
