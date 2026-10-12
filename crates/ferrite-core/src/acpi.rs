//! Bounded ACPI input, namespace inspection and OS-aware SSDTs compiled by native iasl.
use crate::{config, planner::Plan, report::text, workspace};
use anyhow::{Context, Result, bail};
use plist::Value;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Table {
    pub name: String,
    pub signature: String,
    pub sha256: String,
    pub size: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Device {
    pub path: String,
    pub kind: String,
    pub hid: Option<String>,
    pub body: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AcpiResult {
    pub tables: Vec<Table>,
    pub generated: Vec<String>,
    pub notices: Vec<String>,
}
#[derive(Clone, Debug)]
struct Token {
    value: String,
    start: usize,
    end: usize,
}
fn tokens(source: &str) -> Result<Vec<Token>> {
    let b = source.as_bytes();
    let mut out = vec![];
    let mut i = 0;
    while i < b.len() {
        if b[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if b.get(i..i + 2) == Some(b"//") {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if b.get(i..i + 2) == Some(b"/*") {
            i += 2;
            while i + 1 < b.len() && b.get(i..i + 2) != Some(b"*/") {
                i += 1;
            }
            if i + 1 >= b.len() {
                bail!("Незавершённый комментарий ASL");
            }
            i += 2;
            continue;
        }
        let start = i;
        if b[i] == b'"' {
            i += 1;
            while i < b.len() {
                if b[i] == b'\\' {
                    i += 2;
                    continue;
                }
                if b[i] == b'"' {
                    i += 1;
                    break;
                }
                i += 1;
            }
        } else if b[i].is_ascii_alphanumeric() || matches!(b[i], b'_' | b'\\' | b'^') {
            i += 1;
            while i < b.len()
                && (b[i].is_ascii_alphanumeric() || matches!(b[i], b'_' | b'.' | b'\\' | b'^'))
            {
                i += 1;
            }
        } else {
            i += source[i..]
                .chars()
                .next()
                .context("ASL character")?
                .len_utf8();
        }
        if i > b.len() {
            bail!("Некорректная строка ASL");
        }
        out.push(Token {
            value: source[start..i].into(),
            start,
            end: i,
        });
    }
    Ok(out)
}
fn qualified(parent: &str, name: &str) -> String {
    if name.starts_with('\\') {
        return name.into();
    }
    let mut parent = parent.to_string();
    let mut name = name;
    while let Some(rest) = name.strip_prefix('^') {
        parent = parent
            .rsplit_once('.')
            .map(|(p, _)| p.to_string())
            .unwrap_or_else(|| "\\".into());
        name = rest;
    }
    if parent == "\\" {
        format!("\\{name}")
    } else {
        format!("{parent}.{name}")
    }
}
fn matching(t: &[Token], start: usize, left: &str, right: &str) -> Result<usize> {
    let mut depth = 0;
    for (i, token) in t.iter().enumerate().skip(start) {
        if token.value == left {
            depth += 1;
        }
        if token.value == right {
            depth -= 1;
            if depth == 0 {
                return Ok(i);
            }
        }
    }
    bail!("Незавершённая структура ASL")
}
pub fn namespace(source: &str) -> Result<Vec<Device>> {
    if source.len() > 16 * 1024 * 1024 {
        bail!("ASL слишком велик");
    }
    let t = tokens(source)?;
    let mut out = vec![];
    fn scope(
        source: &str,
        t: &[Token],
        start: usize,
        end: usize,
        parent: &str,
        depth: usize,
        out: &mut Vec<Device>,
    ) -> Result<()> {
        if depth > 64 {
            bail!("ACPI namespace слишком глубок");
        }
        let hid_re = Regex::new(r#"Name\s*\(\s*_HID\s*,\s*(?:EisaId\s*\(\s*)?"([A-Z0-9]+)""#)?;
        let mut i = start;
        while i < end {
            let kind = t[i].value.as_str();
            if matches!(kind, "Scope" | "Device" | "Processor")
                && t.get(i + 1).is_some_and(|v| v.value == "(")
            {
                let close = matching(t, i + 1, "(", ")")?;
                if t.get(close + 1).is_none_or(|v| v.value != "{") {
                    i = close + 1;
                    continue;
                }
                let finish = matching(t, close + 1, "{", "}")?;
                let name = &t.get(i + 2).context("ACPI name")?.value;
                let path = qualified(parent, name);
                let begin_body = t[close + 1].end;
                let end_body = t[finish].start;
                // Exclude nested named scopes from own attributes to avoid mistaking a bridge for its EC child.
                let mut body = source[begin_body..end_body].to_string();
                let mut j = close + 2;
                let mut ranges = vec![];
                while j < finish {
                    if matches!(t[j].value.as_str(), "Scope" | "Device" | "Processor")
                        && t.get(j + 1).is_some_and(|v| v.value == "(")
                    {
                        let c = matching(t, j + 1, "(", ")")?;
                        if t.get(c + 1).is_some_and(|v| v.value == "{") {
                            let e = matching(t, c + 1, "{", "}")?;
                            ranges.push((t[j].start - begin_body, t[e].end - begin_body));
                            j = e + 1;
                            continue;
                        }
                    }
                    j += 1;
                }
                for (a, b) in ranges.into_iter().rev() {
                    body.replace_range(a..b, &" ".repeat(b - a));
                }
                if kind != "Scope" {
                    let hid = hid_re.captures(&body).map(|c| c[1].into());
                    out.push(Device {
                        path: path.clone(),
                        kind: kind.into(),
                        hid,
                        body,
                    });
                }
                scope(source, t, close + 2, finish, &path, depth + 1, out)?;
                i = finish + 1;
            } else {
                i += 1;
            }
        }
        Ok(())
    }
    scope(source, &t, 0, t.len(), "\\", 0, &mut out)?;
    Ok(out)
}
pub fn validate_table(raw: &[u8], name: &str) -> Result<Table> {
    if raw.len() < 36 {
        bail!("{name}: короткий заголовок ACPI");
    }
    let len = u32::from_le_bytes(raw[4..8].try_into()?);
    if len as usize != raw.len() || raw.len() > 16 * 1024 * 1024 {
        bail!("{name}: размер ACPI не соответствует заголовку");
    }
    if &raw[..4] != b"FACS" && raw.iter().fold(0u8, |a, b| a.wrapping_add(*b)) != 0 {
        bail!("{name}: неверная контрольная сумма ACPI");
    }
    let sig = std::str::from_utf8(&raw[..4])?.to_string();
    if !raw[..4]
        .iter()
        .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
    {
        bail!("{name}: неизвестная ACPI-сигнатура");
    }
    Ok(Table {
        name: name.into(),
        signature: sig,
        sha256: format!("{:x}", sha2::Sha256::digest(raw)),
        size: len,
    })
}
use sha2::Digest;
fn declaration(name: &str, body: &str) -> String {
    let name = name.strip_prefix("SSDT-").unwrap_or(name);
    let name = &name[..name.len().min(8)];
    format!(
        "DefinitionBlock (\"\", \"SSDT\", 2, \"FERRIT\", \"{name}\", 0x00000001)\n{{\n{body}\n}}\n"
    )
}
fn external(path: &str, kind: &str) -> String {
    format!("External ({path}, {kind})\n")
}
fn valid_path(path: &str) -> bool {
    Regex::new(r"^\\[A-Z0-9_]{1,4}(\.[A-Z0-9_]{1,4})*$").is_ok_and(|r| r.is_match(path))
}
pub fn compile(iasl: &str, destination: &Path, name: &str, source: &str) -> Result<PathBuf> {
    if !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
        bail!("Invalid SSDT name");
    }
    fs::create_dir_all(destination)?;
    let dsl = destination.join(format!("{name}.dsl"));
    let prefix = destination.join(name);
    fs::write(&dsl, source)?;
    workspace::run_bounded(
        iasl,
        &[
            "-ve",
            "-p",
            prefix.to_str().context("iasl path")?,
            dsl.to_str().context("DSL path")?,
        ],
        Duration::from_secs(30),
    )?;
    let aml = destination.join(format!("{name}.aml"));
    validate_table(&fs::read(&aml)?, name)?;
    Ok(aml)
}
pub fn generate(
    input: &Path,
    output: &Path,
    iasl: &str,
    plan: &Plan,
    cfg: &mut Value,
) -> Result<AcpiResult> {
    let mut tables = vec![];
    let mut dsdt = None;
    let temp = tempfile::tempdir()?;
    let mut dependencies = vec![];
    for e in fs::read_dir(input)? {
        let e = e?;
        if e.file_type()?.is_symlink() {
            bail!("Ссылка в папке ACPI");
        }
        let p = e.path();
        if !p.is_file() {
            continue;
        }
        if !matches!(p.extension().and_then(|s| s.to_str()), Some("aml" | "dat")) {
            continue;
        }
        if fs::metadata(&p)?.len() > 16 * 1024 * 1024 {
            bail!("ACPI table слишком велика");
        }
        let raw = fs::read(&p)?;
        let table = validate_table(&raw, &e.file_name().to_string_lossy())?;
        let copied = temp.path().join(e.file_name());
        fs::write(&copied, &raw)?;
        if table.signature == "DSDT" {
            if dsdt.replace(copied.clone()).is_some() {
                bail!("В папке больше одного DSDT");
            }
        } else if table.signature == "SSDT" {
            dependencies.push(copied);
        }
        tables.push(table);
    }
    let dsdt = dsdt.context("Сборка требует корректную таблицу DSDT")?;
    let prefix = temp.path().join("DSDT");
    let mut args = vec![
        "-dl".to_string(),
        "-p".into(),
        prefix.to_string_lossy().into_owned(),
    ];
    if !dependencies.is_empty() {
        args.push("-e".into());
        args.extend(
            dependencies
                .iter()
                .map(|p| p.to_string_lossy().into_owned()),
        );
    }
    args.push("-d".into());
    args.push(dsdt.to_string_lossy().into_owned());
    let argv: Vec<_> = args.iter().map(String::as_str).collect();
    workspace::run_bounded(iasl, &argv, Duration::from_secs(60))?;
    let source = fs::read_to_string(prefix.with_extension("dsl"))?;
    let devices = namespace(&source)?;
    let lpc = devices
        .iter()
        .find(|d| d.hid.as_deref() == Some("PNP0A08"))
        .and_then(|root| {
            devices.iter().find(|d| {
                d.path.starts_with(&root.path)
                    && ["LPCB", "LPC0", "LPC_", "SBRG", "ISA_"]
                        .iter()
                        .any(|n| d.path.ends_with(n))
            })
        })
        .or_else(|| {
            devices.iter().find(|d| {
                ["LPCB", "LPC0", "LPC_", "SBRG", "ISA_"]
                    .iter()
                    .any(|n| d.path.ends_with(n))
            })
        });
    let mut result = AcpiResult {
        tables,
        generated: vec![],
        notices: vec![],
    };
    let mut add = vec![];
    let mut emit = |name: &str, body: &str| -> Result<()> {
        compile(iasl, output, name, &declaration(name, body))?;
        add.push(config::json(serde_json::json!({"Comment":format!("Ferrite {name}"),"Enabled":true,"Path":format!("{name}.aml")}))?);
        result.generated.push(name.into());
        Ok(())
    };
    let has_ec = devices
        .iter()
        .any(|d| d.path.ends_with(".EC") || d.path.ends_with(".EC_"));
    let laptop = plan.hardware["Motherboard"]["Platform"] == "Laptop";
    if !has_ec {
        let lpc = lpc.context(
            "Не найден LPC bridge для FakeEC; импортируйте SSDT-EC вручную вместо угадывания пути",
        )?;
        if !valid_path(&lpc.path) {
            bail!("Invalid LPC path");
        }
        let body = format!(
            "{}Scope ({}) {{ Device (EC) {{ Name (_HID, \"ACID0001\") Method (_STA, 0, NotSerialized) {{ If (_OSI (\"Darwin\")) {{ Return (0x0F) }} Return (Zero) }} }} }}",
            external(&lpc.path, "DeviceObj"),
            lpc.path
        );
        emit("SSDT-EC", &body)?;
    } else {
        result
            .notices
            .push("EC уже есть в DSDT; повторный EC не создаётся".into());
    }
    if !devices.iter().any(|d| d.path.ends_with(".USBX")) {
        emit(
            "SSDT-USBX",
            r#"Scope (\_SB) { Device (USBX) { Name (_ADR, Zero) Method (_DSM, 4, NotSerialized) { If (!Arg2) { Return (Buffer (One) { 0x03 }) } Return (Package () { "kUSBSleepPowerSupply", 0x13EC, "kUSBSleepPortCurrentLimit", 0x0834, "kUSBWakePowerSupply", 0x13EC, "kUSBWakePortCurrentLimit", 0x0834 }) } } }"#,
        )?;
    }
    if plan.hardware["CPU"]["Manufacturer"]
        .as_str()
        .is_some_and(|s| s.contains("Intel"))
    {
        if let Some(cpu) = devices
            .iter()
            .find(|d| d.kind == "Processor" || d.hid.as_deref() == Some("ACPI0007"))
        {
            if cpu.body.contains("_DSM") {
                bail!("CPU уже содержит _DSM: требуется персональный PLUG SSDT");
            }
            if valid_path(&cpu.path) {
                emit(
                    "SSDT-PLUG",
                    &format!(
                        "{}Scope ({}) {{ Method (_DSM, 4, NotSerialized) {{ If (!Arg2) {{ Return (Buffer (One) {{ 0x03 }}) }} Return (Package () {{ \"plugin-type\", One }}) }} }}",
                        external(
                            &cpu.path,
                            if cpu.kind == "Processor" {
                                "ProcessorObj"
                            } else {
                                "DeviceObj"
                            }
                        ),
                        cpu.path
                    ),
                )?;
            }
        } else {
            bail!("Не найден CPU namespace для PLUG");
        }
    }
    if devices.iter().any(|d| d.hid.as_deref() == Some("ACPI000E")) {
        if source.contains("Name (STAS,") {
            let path = devices
                .iter()
                .find(|d| d.body.contains("Name (STAS,"))
                .map(|d| format!("{}.STAS", d.path))
                .context("STAS scope не определён: нужен персональный RTC SSDT")?;
            if valid_path(&path) {
                emit(
                    "SSDT-AWAC",
                    &format!(
                        "{}Scope (\\) {{ Method (_INI, 0, NotSerialized) {{ If (_OSI (\"Darwin\")) {{ {} = One }} }} }}",
                        external(&path, "IntObj"),
                        path
                    ),
                )?;
            }
        } else {
            bail!(
                "AWAC обнаружен без подтверждённой STAS-переменной; требуется персональный RTC SSDT"
            );
        }
    }
    if laptop {
        result.notices.push("PNLF, battery, IRQ и I2C требуют персональной проверки; автоматический перенос этих патчей ещё не завершён".into());
    }
    for name in &plan.disabled {
        for section in ["GPU", "Network", "SD Controller"] {
            if let Some(p) = plan.hardware[section].get(name) {
                let path = text(p, "ACPI Path");
                if valid_path(path) {
                    result.notices.push(format!("{name}: аппаратный путь {path}; GPU выключается через DeviceProperties, прочие устройства не объявляются отключёнными без персональной ACPI patch"));
                }
            }
        }
    }
    config::set(cfg, &["ACPI", "Add"], Value::Array(add))?;
    Ok(result)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parent_hid_does_not_inherit_child() {
        let s = r#"Scope (\_SB) { Device (PCI0) { Name (_HID, EisaId ("PNP0A08")) Device (LPCB) { Device (H_EC) { Name (_HID, EisaId ("PNP0C09")) } } } }"#;
        let d = namespace(s).unwrap();
        assert_eq!(d[0].hid.as_deref(), Some("PNP0A08"));
        assert_eq!(d[1].hid, None);
        assert_eq!(d[2].path, "\\_SB.PCI0.LPCB.H_EC");
    }
    #[test]
    fn comments_and_strings_do_not_create_devices() {
        let d = namespace(
            r#"// Device (FAKE) {}
Device (REAL) { Name (TEST, "Device (FAKE) { } ") }"#,
        )
        .unwrap();
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].path, "\\REAL");
    }
    #[test]
    fn checksum_failure_rejected() {
        assert!(validate_table(&[0; 36], "bad").is_err());
    }
}
