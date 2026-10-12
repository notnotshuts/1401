use anyhow::{Context, Result, bail};
use plist::{Dictionary, Value};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs,
    path::{Component, Path, PathBuf},
    time::Duration,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Audit {
    pub root: PathBuf,
    pub issues: Vec<String>,
    pub warnings: Vec<String>,
    pub files: usize,
    pub config_sha256: String,
    pub validator_passed: bool,
    pub ready: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Change {
    pub rule: String,
    pub before: String,
    pub after: String,
    pub reason: String,
}

fn safe_file(root: &Path, relative: &str) -> Result<PathBuf> {
    // Backslashes are also rejected on Unix so imported Windows paths cannot hide traversal.
    if relative.contains('\\')
        || relative.contains(':')
        || relative.is_empty()
        || Path::new(relative)
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        bail!("Небезопасный EFI-путь: {relative}");
    }
    let root = fs::canonicalize(root)?;
    let mut current = root.clone();
    for component in Path::new(relative).components() {
        current.push(component);
        if fs::symlink_metadata(&current)?.file_type().is_symlink() {
            bail!("EFI содержит ссылку: {}", current.display());
        }
    }
    if !fs::canonicalize(&current)?.starts_with(&root) {
        bail!("EFI-путь выходит за рабочую папку");
    }
    Ok(current)
}
fn collect(root: &Path, dir: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    if files.len() > 10000 {
        bail!("В EFI слишком много файлов");
    }
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            bail!("EFI содержит ссылку: {}", path.display());
        }
        if kind.is_dir() {
            collect(root, &path, files)?;
        } else if kind.is_file() {
            if !path.starts_with(root) {
                bail!("Путь вне EFI");
            }
            files.push(path);
        } else {
            bail!("EFI содержит специальный файл");
        }
    }
    Ok(())
}
pub fn audit(root: &Path, validator: Option<(&Path, &str)>) -> Result<Audit> {
    let root = fs::canonicalize(root).context("Выберите папку EFI с BOOT и OC")?;
    let mut result = Audit {
        root: root.clone(),
        issues: vec![],
        warnings: vec![],
        files: 0,
        config_sha256: String::new(),
        validator_passed: false,
        ready: false,
    };
    let mut files = vec![];
    collect(&root, &root, &mut files)?;
    result.files = files.len();
    for relative in ["BOOT/BOOTx64.efi", "OC/OpenCore.efi", "OC/config.plist"] {
        if let Err(e) = safe_file(&root, relative) {
            result.issues.push(format!("{relative}: {e}"));
        }
    }
    let config_path = safe_file(&root, "OC/config.plist")?;
    if fs::metadata(&config_path)?.len() > 16 * 1024 * 1024 {
        bail!("config.plist превышает 16 MiB");
    }
    let raw = fs::read(&config_path)?;
    result.config_sha256 = format!("{:x}", Sha256::digest(&raw));
    let cfg = Value::from_reader(std::io::Cursor::new(raw))
        .context("Не удалось прочитать config.plist")?;
    let dict = cfg
        .as_dictionary()
        .context("Корень config.plist должен быть словарём")?;
    for section in [
        "ACPI",
        "Booter",
        "DeviceProperties",
        "Kernel",
        "Misc",
        "NVRAM",
        "PlatformInfo",
        "UEFI",
    ] {
        if dict.get(section).and_then(Value::as_dictionary).is_none() {
            result.issues.push(format!("Отсутствует раздел {section}"));
        }
    }
    for (section, key, path_key, prefix) in [
        ("ACPI", "Add", "Path", "OC/ACPI"),
        ("Kernel", "Add", "BundlePath", "OC/Kexts"),
        ("UEFI", "Drivers", "Path", "OC/Drivers"),
    ] {
        let Some(rows) = dict
            .get(section)
            .and_then(Value::as_dictionary)
            .and_then(|d| d.get(key))
            .and_then(Value::as_array)
        else {
            result.issues.push(format!("Нет массива {section}/{key}"));
            continue;
        };
        let mut seen = BTreeSet::new();
        for row in rows {
            let Some(r) = row.as_dictionary() else {
                result
                    .issues
                    .push(format!("Некорректная запись {section}/{key}"));
                continue;
            };
            if r.get("Enabled").and_then(Value::as_boolean) == Some(false) {
                continue;
            }
            let Some(relative) = r.get(path_key).and_then(Value::as_string) else {
                result
                    .issues
                    .push(format!("Нет {path_key} в {section}/{key}"));
                continue;
            };
            if !seen.insert(relative) {
                result
                    .issues
                    .push(format!("Дубликат {section}: {relative}"));
            }
            if let Err(e) = safe_file(&root, &format!("{prefix}/{relative}")) {
                result.issues.push(e.to_string());
            }
            if section == "Kernel" {
                for k in ["PlistPath", "ExecutablePath"] {
                    if let Some(file) = r
                        .get(k)
                        .and_then(Value::as_string)
                        .filter(|s| !s.is_empty())
                        && let Err(e) = safe_file(&root, &format!("{prefix}/{relative}/{file}"))
                    {
                        result.issues.push(e.to_string());
                    }
                }
            }
        }
    }
    if let Some((exe, expected)) = validator {
        if expected.len() != 64 || !expected.bytes().all(|b| b.is_ascii_hexdigit()) {
            bail!("Укажите полный SHA-256 доверенного ocvalidate");
        }
        if fs::symlink_metadata(exe)?.file_type().is_symlink()
            || fs::metadata(exe)?.len() > 64 * 1024 * 1024
        {
            bail!("Недопустимый валидатор");
        }
        let digest = format!("{:x}", Sha256::digest(fs::read(exe)?));
        if digest != expected.to_lowercase() {
            bail!("SHA-256 ocvalidate не совпадает");
        }
        let output = crate::workspace::run_bounded(
            exe.to_str().context("Путь ocvalidate не UTF-8")?,
            &[config_path.to_str().context("Путь config не UTF-8")?],
            Duration::from_secs(30),
        );
        match output {
            Ok(out) => {
                result.validator_passed = true;
                result
                    .warnings
                    .push(String::from_utf8_lossy(&out).trim().to_string());
            }
            Err(e) => result.issues.push(format!("ocvalidate: {e}")),
        }
    } else {
        result.warnings.push("Структурная проверка выполнена. Полная проверка требует ocvalidate из той же версии OpenCore и его доверенного SHA-256.".into());
    }
    result.ready = result.issues.is_empty() && result.validator_passed;
    Ok(result)
}
fn dictionary<'a>(parent: &'a mut Dictionary, key: &str) -> Result<&'a mut Dictionary> {
    if !parent.contains_key(key) {
        parent.insert(key.into(), Value::Dictionary(Dictionary::new()));
    }
    parent
        .get_mut(key)
        .unwrap()
        .as_dictionary_mut()
        .with_context(|| format!("{key} должен быть словарём"))
}
/// Conservative policies that don't depend on guessing motherboard or GPU topology.
pub fn repair_copy(root: &Path, output: &Path) -> Result<Vec<Change>> {
    let root = fs::canonicalize(root)?;
    if output.exists() {
        bail!("Выходная папка уже существует; исходная EFI сохраняется");
    }
    let parent = output.parent().context("Нужна родительская папка")?;
    fs::create_dir_all(parent)?;
    if fs::canonicalize(parent)?.starts_with(&root) {
        bail!("Копия EFI должна находиться вне исходной EFI");
    }
    let mut files = vec![];
    collect(&root, &root, &mut files)?;
    let config = safe_file(&root, "OC/config.plist")?;
    let mut cfg = Value::from_file(config)?;
    let top = cfg.as_dictionary_mut().context("Некорректная EFI")?;
    let mut changes = vec![];
    for (section, key) in [("ACPI", "Path"), ("Kernel", "BundlePath")] {
        if let Some(rows) = dictionary(top, section)?
            .get_mut("Add")
            .and_then(Value::as_array_mut)
        {
            let mut seen = BTreeSet::new();
            let before = rows.len();
            rows.retain(|row| {
                row.as_dictionary()
                    .and_then(|d| d.get(key))
                    .and_then(Value::as_string)
                    .is_none_or(|s| seen.insert(s.to_string()))
            });
            if before != rows.len() {
                changes.push(Change {
                    rule: "dedupe-add".into(),
                    before: before.to_string(),
                    after: rows.len().to_string(),
                    reason: format!("Удалены дубликаты {section}/{key}; первая запись сохранена"),
                });
            }
        }
    }
    let nv = dictionary(top, "NVRAM")?;
    let del = dictionary(nv, "Delete")?;
    if let Some(rows) = del
        .get_mut("7C436110-AB2A-4BBB-A880-FE41995C9F82")
        .and_then(Value::as_array_mut)
    {
        let n = rows.len();
        rows.retain(|r| r.as_string() != Some("boot-args"));
        if rows.len() != n {
            changes.push(Change {
                rule: "keep-boot-args".into(),
                before: "NVRAM/Delete/boot-args".into(),
                after: "Сохранить имеющиеся boot-args".into(),
                reason: "Не сбрасывать флаги уже установленной macOS".into(),
            });
        }
    }
    // Copy into a fresh directory, never delete/replace the user's source or an existing destination.
    fs::create_dir(output)?;
    for file in files {
        let relative = file.strip_prefix(&root)?;
        let to = output.join(relative);
        if let Some(p) = to.parent() {
            fs::create_dir_all(p)?;
        }
        fs::copy(file, to)?;
    }
    cfg.to_file_xml(output.join("OC/config.plist"))?;
    Ok(changes)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(root: &Path) {
        fs::create_dir_all(root.join("BOOT")).unwrap();
        fs::create_dir_all(root.join("OC/ACPI")).unwrap();
        fs::write(root.join("BOOT/BOOTx64.efi"), b"fixture").unwrap();
        fs::write(root.join("OC/OpenCore.efi"), b"fixture").unwrap();
        fs::write(root.join("OC/ACPI/Test.aml"), b"fixture").unwrap();
        let cfg=Value::from_reader(std::io::Cursor::new(br#"<?xml version="1.0" encoding="UTF-8"?><plist version="1.0"><dict>
        <key>ACPI</key><dict><key>Add</key><array><dict><key>Path</key><string>Test.aml</string><key>Enabled</key><true/></dict><dict><key>Path</key><string>Test.aml</string><key>Enabled</key><true/></dict></array></dict>
        <key>Booter</key><dict/><key>DeviceProperties</key><dict/>
        <key>Kernel</key><dict><key>Add</key><array/></dict><key>Misc</key><dict/>
        <key>NVRAM</key><dict><key>Delete</key><dict><key>7C436110-AB2A-4BBB-A880-FE41995C9F82</key><array><string>boot-args</string><string>csr-active-config</string></array></dict></dict>
        <key>PlatformInfo</key><dict/><key>UEFI</key><dict><key>Drivers</key><array/></dict>
        </dict></plist>"#)).unwrap();
        cfg.to_file_xml(root.join("OC/config.plist")).unwrap();
    }
    #[test]
    fn repair_preserves_original_and_requires_validation() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("EFI");
        fixture(&source);
        let original = fs::read(source.join("OC/config.plist")).unwrap();
        assert!(
            audit(&source, None)
                .unwrap()
                .issues
                .iter()
                .any(|x| x.contains("Дубликат"))
        );
        let output = temp.path().join("repaired");
        let changes = repair_copy(&source, &output).unwrap();
        assert_eq!(changes.len(), 2);
        assert_eq!(fs::read(source.join("OC/config.plist")).unwrap(), original);
        let result = audit(&output, None).unwrap();
        assert!(result.issues.is_empty());
        assert!(!result.ready);
        let cfg = Value::from_file(output.join("OC/config.plist")).unwrap();
        let nv = cfg.as_dictionary().unwrap()["NVRAM"]
            .as_dictionary()
            .unwrap();
        let rows = nv["Delete"].as_dictionary().unwrap()["7C436110-AB2A-4BBB-A880-FE41995C9F82"]
            .as_array()
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].as_string(), Some("csr-active-config"));
    }
    #[test]
    fn validator_digest_mismatch_stops_execution() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("EFI");
        fixture(&source);
        let executable = temp.path().join("validator");
        fs::write(&executable, b"should never run").unwrap();
        assert!(
            audit(&source, Some((&executable, &"0".repeat(64))))
                .unwrap_err()
                .to_string()
                .contains("SHA-256")
        );
    }
    #[test]
    fn traversal_and_windows_paths_rejected() {
        let temp = tempfile::tempdir().unwrap();
        for p in ["../config.plist", "/tmp/config.plist", "OC\\..\\a", "C:/x"] {
            assert!(safe_file(temp.path(), p).is_err());
        }
    }
    #[cfg(unix)]
    #[test]
    fn symlink_escape_rejected() {
        let temp = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink("/etc", temp.path().join("OC")).unwrap();
        assert!(safe_file(temp.path(), "OC/passwd").is_err());
    }
    #[test]
    fn existing_output_never_overwritten() {
        let temp = tempfile::tempdir().unwrap();
        assert!(repair_copy(temp.path(), temp.path()).is_err());
    }
}
