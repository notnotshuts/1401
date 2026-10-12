use crate::{
    acpi, config, downloads, efi,
    planner::{self, Options, Plan},
    report::Report,
    workspace,
};
use anyhow::{Context, Result, bail};
use plist::Value;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    time::Duration,
};
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Build {
    pub schema: u32,
    pub ok: bool,
    pub output: PathBuf,
    pub plan: Plan,
    pub acpi: acpi::AcpiResult,
    pub audit: efi::Audit,
    pub files: BTreeMap<String, String>,
    pub bios_guide: PathBuf,
}
fn named(root: &Path, name: &str) -> Result<PathBuf> {
    downloads::files(root, name)?
        .into_iter()
        .find(|p| {
            p.file_name().is_some_and(|n| n == name)
                && !p.components().any(|c| c.as_os_str() == "__MACOSX")
        })
        .with_context(|| format!("Компонент {name} не найден"))
}
fn chmod_executable(_path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(_path, fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}
#[derive(Clone)]
struct Bundle {
    path: String,
    id: String,
    libraries: Vec<String>,
    executable: String,
    plist: String,
    min: String,
    max: String,
}
fn bundles(root: &Path, plan: &Plan) -> Result<Vec<Bundle>> {
    let mut out = vec![];
    for path in downloads::files(root, ".kext")? {
        let relative = path
            .strip_prefix(root)?
            .to_string_lossy()
            .replace('\\', "/");
        let plist_relative = if path.join("Contents/Info.plist").is_file() {
            "Contents/Info.plist"
        } else if path.join("Info.plist").is_file() {
            "Info.plist"
        } else {
            bail!("Kext {relative} без Info.plist");
        };
        let info_path = path.join(plist_relative);
        let info = Value::from_file(info_path)?;
        let info = info.as_dictionary().context("Kext Info.plist")?;
        let id = info
            .get("CFBundleIdentifier")
            .and_then(Value::as_string)
            .context("Kext без BundleIdentifier")?
            .to_string();
        let executable = info
            .get("CFBundleExecutable")
            .and_then(Value::as_string)
            .unwrap_or("");
        if executable.contains(['/', '\\']) || executable == ".." {
            bail!("Небезопасный CFBundleExecutable");
        }
        let executable = if executable.is_empty() {
            String::new()
        } else if plist_relative == "Contents/Info.plist" {
            format!("Contents/MacOS/{executable}")
        } else {
            executable.to_string()
        };
        if !executable.is_empty() && !path.join(&executable).is_file() {
            bail!("В kext {relative} отсутствует executable");
        }
        let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        let data = plan.kexts.iter().find(|k| k.name == name);
        let libraries = info
            .get("OSBundleLibraries")
            .and_then(Value::as_dictionary)
            .map(|d| d.keys().cloned().collect())
            .unwrap_or_default();
        out.push(Bundle {
            path: relative,
            id,
            libraries,
            executable,
            plist: plist_relative.into(),
            min: data.map(|k| k.min.clone()).unwrap_or_default(),
            max: data
                .map(|k| k.max.clone())
                .filter(|v| v != "25.99.99")
                .unwrap_or_default(),
        });
    }
    Ok(out)
}
fn order_bundles(bundles: Vec<Bundle>) -> Result<Vec<Value>> {
    let mut map = BTreeMap::new();
    for b in &bundles {
        if map.insert(b.id.clone(), b).is_some() {
            bail!("Повторяющийся BundleIdentifier {}", b.id);
        }
    }
    let mut result = vec![];
    let mut seen = BTreeSet::new();
    let mut visiting = BTreeSet::new();
    fn visit<'a>(
        id: &'a str,
        map: &BTreeMap<String, &'a Bundle>,
        seen: &mut BTreeSet<String>,
        visiting: &mut BTreeSet<String>,
        result: &mut Vec<Value>,
    ) -> Result<()> {
        if seen.contains(id) {
            return Ok(());
        }
        if !visiting.insert(id.into()) {
            bail!("Цикл OSBundleLibraries: {id}");
        }
        let b = map[id];
        for lib in &b.libraries {
            if map.contains_key(lib) {
                visit(lib, map, seen, visiting, result)?;
            } else if !lib.starts_with("com.apple.") {
                bail!("Недостающая kext dependency {lib} для {id}");
            }
        }
        result.push(config::json(serde_json::json!({"Arch":"Any","BundlePath":b.path,"Comment":b.id,"Enabled":true,"ExecutablePath":b.executable,"MaxKernel":b.max,"MinKernel":b.min,"PlistPath":b.plist}))?);
        visiting.remove(id);
        seen.insert(id.into());
        Ok(())
    }
    for b in &bundles {
        visit(&b.id, &map, &mut seen, &mut visiting, &mut result)?;
    }
    Ok(result)
}
fn append_patch(cfg: &mut Value, patch: Value) -> Result<()> {
    let mut patches = config::get(cfg, &["Kernel", "Patch"])
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    patches.push(patch);
    config::set(cfg, &["Kernel", "Patch"], Value::Array(patches))
}
fn amd_patches(cfg: &mut Value, source: &Path, plan: &Plan) -> Result<()> {
    let patches = Value::from_file(source)?;
    let rows = config::get(&patches, &["Kernel", "Patch"])
        .and_then(Value::as_array)
        .or_else(|| patches.as_array())
        .context("AMD kernel patches")?;
    let cores = planner::number(&plan.hardware["CPU"], "Core Count")
        .context("AMD patch requires physical Core Count")?;
    if cores == 0 || cores > 255 {
        bail!("AMD core count вне диапазона патча");
    }
    for patch in rows {
        let mut patch = patch.clone();
        let d = patch.as_dictionary_mut().context("AMD patch")?;
        let comment = d
            .get("Comment")
            .and_then(Value::as_string)
            .unwrap_or("")
            .to_string();
        if comment.contains("cpuid_cores_per_package") {
            let mut data = d
                .get("Replace")
                .and_then(Value::as_data)
                .context("AMD Replace bytes")?
                .to_vec();
            if data.len() < 2 {
                bail!("AMD cores patch слишком короткий");
            }
            data[1] = cores as u8;
            d.insert("Replace".into(), Value::Data(data));
        }
        if comment.contains("_mtrr_update_action") {
            let chipset = plan.hardware["Motherboard"]["Chipset"]
                .as_str()
                .unwrap_or("");
            if chipset.to_uppercase().contains("TRX") {
                d.insert("Enabled".into(), Value::Boolean(false));
            } else if plan.hardware["GPU"]
                .as_object()
                .is_some_and(|g| g.values().any(|p| p["Manufacturer"] == "AMD"))
            {
                if comment.to_lowercase().contains("algrey") {
                    d.insert("Enabled".into(), Value::Boolean(false));
                }
                if comment.to_lowercase().contains("shaneee") {
                    d.insert("Enabled".into(), Value::Boolean(true));
                }
            }
        }
        append_patch(cfg, patch)?;
    }
    Ok(())
}
pub fn guide(plan: &Plan) -> String {
    crate::guide::render(Some(plan), crate::i18n::language())
}

pub fn build(
    report_path: &Path,
    acpi_dir: &Path,
    output: &Path,
    cache: &Path,
    iasl: &str,
    options: &Options,
    progress: &mut impl FnMut(&str, u64, u64),
) -> Result<Build> {
    let report = Report::load(report_path)?;
    let plan = planner::plan(&report, options)?;
    if plan.hardware["BIOS"]["Firmware Type"]
        .as_str()
        .is_some_and(|s| s == "Legacy")
    {
        bail!("Переключите BIOS в UEFI и повторите сканирование; Legacy boot не создаётся");
    }
    if output.exists() {
        bail!(
            "Сборка не перезаписывает существующую папку: {}",
            output.display()
        );
    }
    let temp = tempfile::tempdir()?;
    let mut extracted = BTreeMap::new();
    for (i, a) in plan.artifacts.iter().enumerate() {
        progress(&format!("Загрузка {}", a.name), 0, a.size);
        let archive = downloads::fetch(a, cache, progress)?;
        let dest = temp.path().join(format!("artifact-{i}"));
        if a.url.ends_with(".zip") {
            downloads::extract_zip(&archive, &dest, &a.sha256)?;
        } else {
            fs::create_dir(&dest)?;
            fs::copy(archive, dest.join("patches.plist"))?;
        }
        extracted.insert(a.url.clone(), dest);
    }
    let core_artifact = plan.artifacts.first().context("OpenCore artifact")?;
    let core = &extracted[&core_artifact.url];
    let sample = named(core, "Sample.plist")?;
    let core_efi = core.join("X64/EFI");
    if !core_efi.is_dir() {
        bail!("Закреплённый OpenCore archive без X64/EFI");
    }
    let staging = temp.path().join("result");
    let efi_root = staging.join("EFI");
    fs::create_dir(&staging)?;
    downloads::copy_tree(&core_efi, &efi_root)?;
    // Remove release sample tools/drivers/kexts; only selected components are loaded and copied.
    for directory in ["OC/Drivers", "OC/Kexts", "OC/ACPI", "OC/Tools"] {
        let p = efi_root.join(directory);
        if p.exists() {
            fs::remove_dir_all(&p)?;
        }
        fs::create_dir_all(p)?;
    }
    let binary_data_artifact = plan
        .artifacts
        .iter()
        .find(|a| a.name == "OcBinaryData")
        .context("OcBinaryData")?;
    let binary_data = &extracted[&binary_data_artifact.url];
    let hfs = named(binary_data, "HfsPlus.efi")?;
    let mut drivers = vec![];
    for name in ["OpenRuntime.efi", "ResetNvramEntry.efi", "HfsPlus.efi"] {
        let source = if name == "HfsPlus.efi" {
            hfs.clone()
        } else {
            named(&core_efi, name)?
        };
        fs::copy(source, efi_root.join("OC/Drivers").join(name))?;
        drivers.push(config::json(serde_json::json!({"Arguments":"","Comment":"","Enabled":true,"LoadEarly":false,"HideVerbose":false,"Path":name}))?);
    }
    if plan.target == 25 {
        let apfs =
            named(binary_data, "apfs_aligned.efi").or_else(|_| named(core, "apfs_aligned.efi"))?;
        fs::copy(apfs, efi_root.join("OC/Drivers/apfs_aligned.efi"))?;
        drivers.push(config::json(serde_json::json!({"Arguments":"","Comment":"","Enabled":true,"LoadEarly":false,"HideVerbose":true,"Path":"apfs_aligned.efi"}))?);
    }
    let macserial = named(
        core,
        crate::platform::utility_name("macserial", crate::platform::host())?,
    )?;
    chmod_executable(&macserial)?;
    let raw = workspace::run_bounded(
        macserial.to_str().context("macserial path")?,
        &["-g", "-m", &plan.smbios, "-n", "1"],
        Duration::from_secs(15),
    )?;
    let pairs = String::from_utf8(raw)?;
    let (serial, mlb) = pairs
        .lines()
        .find_map(|l| l.split_once('|'))
        .context("macserial did not generate serial/MLB")?;
    let mut cfg = config::generate(&sample, &plan, options, serial.trim(), mlb.trim())?;
    config::set(&mut cfg, &["UEFI", "Drivers"], Value::Array(drivers))?;
    for k in &plan.kexts {
        let a = planner::kext_artifact(k, plan.target)?;
        let root = &extracted[&a.url];
        let name = format!("{}.kext", k.name);
        let kext = named(root, &name).or_else(|_| {
            if k.name == "Feixiao" {
                named(root, "rtw88.kext")
            } else {
                Err(anyhow::anyhow!("Не найден {name}"))
            }
        })?;
        let destination = efi_root
            .join("OC/Kexts")
            .join(kext.file_name().context("Kext name")?);
        if !destination.exists() {
            downloads::copy_tree(&kext, &destination)?;
        }
    }
    config::set(
        &mut cfg,
        &["Kernel", "Add"],
        Value::Array(order_bundles(bundles(&efi_root.join("OC/Kexts"), &plan)?)?),
    )?;
    let acpi = acpi::generate(acpi_dir, &efi_root.join("OC/ACPI"), iasl, &plan, &mut cfg)?;
    for a in &plan.artifacts {
        if a.name == "AMD Vanilla Patches" {
            amd_patches(&mut cfg, &extracted[&a.url].join("patches.plist"), &plan)?;
        }
    }
    cfg.to_file_xml(efi_root.join("OC/config.plist"))?;
    if Report::load(report_path)?.sha256 != plan.report_sha256 {
        bail!("Report.json изменился во время сборки; результат не публикуется");
    }
    let validator = named(
        core,
        crate::platform::utility_name("ocvalidate", crate::platform::host())?,
    )?;
    chmod_executable(&validator)?;
    let validator_sha = downloads::hash_file(&validator)?;
    let audit = efi::audit(&efi_root, Some((&validator, &validator_sha)))?;
    if !audit.ready {
        bail!("EFI не прошла ocvalidate: {}", audit.issues.join("; "));
    }
    fs::write(staging.join("BIOS.txt"), guide(&plan))?;
    let package = cache.join(&core_artifact.sha256);
    downloads::snapshot_artifact(
        &package,
        core_artifact,
        &staging.join("OpenCorePackage.zip"),
    )?;

    let mut hashes = BTreeMap::new();
    for path in downloads::files(&staging, "")?
        .into_iter()
        .filter(|p| p.is_file())
    {
        hashes.insert(
            path.strip_prefix(&staging)?
                .to_string_lossy()
                .replace('\\', "/"),
            downloads::hash_file(&path)?,
        );
    }
    // Reserve a new output directory once, after validation. Never remove an existing destination.
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::create_dir(output)?;
    downloads::copy_tree(&staging, output)?;
    let mut audit = audit;
    audit.root = output.join("EFI");
    let result = Build {
        schema: 1,
        ok: true,
        output: output.into(),
        plan,
        acpi,
        audit,
        files: hashes,
        bios_guide: output.join("BIOS.txt"),
    };
    let temporary = output.join("build-result.json.part");
    fs::write(&temporary, serde_json::to_vec_pretty(&result)?)?;
    fs::rename(temporary, output.join("build-result.json"))?;
    progress("EFI готова и проверена", 1, 1);
    Ok(result)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn kext_cycle_rejected() {
        let mk = |id: &str, lib: &str| Bundle {
            path: format!("{id}.kext"),
            id: id.into(),
            libraries: vec![lib.into()],
            executable: String::new(),
            plist: "Contents/Info.plist".into(),
            min: String::new(),
            max: String::new(),
        };
        assert!(order_bundles(vec![mk("a", "b"), mk("b", "a")]).is_err());
    }
    #[test]
    fn dependency_before_consumer() {
        let a = Bundle {
            path: "A.kext".into(),
            id: "a".into(),
            libraries: vec![],
            executable: String::new(),
            plist: "Contents/Info.plist".into(),
            min: String::new(),
            max: String::new(),
        };
        let mut b = a.clone();
        b.id = "b".into();
        b.path = "B.kext".into();
        b.libraries = vec!["a".into()];
        let r = order_bundles(vec![b, a]).unwrap();
        assert_eq!(
            config::get(&r[0], &["Comment"]).and_then(Value::as_string),
            Some("a")
        );
    }
}
