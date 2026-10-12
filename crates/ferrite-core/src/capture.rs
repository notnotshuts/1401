#[cfg(target_os = "linux")]
use crate::acpi;
use crate::{report::Report, scan, workspace};
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
use anyhow::bail;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
#[cfg(target_os = "linux")]
use std::io::Read;
use std::{
    fs,
    path::{Path, PathBuf},
};
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    pub report: PathBuf,
    pub acpi: Option<PathBuf>,
    pub notices: Vec<String>,
}
pub fn capture_acpi(output: &Path) -> Result<usize> {
    #[cfg(target_os = "linux")]
    {
        fs::create_dir_all(output)?;
        let mut entries = vec![];
        for root in [
            "/sys/firmware/acpi/tables",
            "/sys/firmware/acpi/tables/dynamic",
        ] {
            if let Ok(files) = fs::read_dir(root) {
                for entry in files {
                    let entry = entry?;
                    if entry.file_type()?.is_file() {
                        entries.push(entry.path());
                    }
                }
            }
        }
        if entries.is_empty() {
            bail!("ACPI tables недоступны; нужны права администратора или импорт дампа");
        }
        let mut dsdt = false;
        let mut count = 0;
        for path in entries {
            let mut raw = vec![];
            fs::File::open(&path)?
                .take(16 * 1024 * 1024 + 1)
                .read_to_end(&mut raw)?;
            let name = path.file_name().context("ACPI filename")?.to_string_lossy();
            let table = acpi::validate_table(&raw, &name)?;
            if table.signature == "DSDT" {
                dsdt = true;
            }
            let destination = output.join(format!("{}-{count}.aml", table.signature));
            if destination.exists() {
                bail!("ACPI capture не перезаписывает файлы");
            }
            fs::write(destination, raw)?;
            count += 1;
        }
        if !dsdt {
            bail!("Дамп не содержит DSDT");
        }
        Ok(count)
    }
    #[cfg(target_os = "windows")]
    {
        crate::windows_scan::capture_acpi(output)
    }
    #[cfg(target_os = "macos")]
    {
        crate::macos::capture_acpi(output)
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
    {
        bail!("ACPI capture доступен только на Windows/Linux")
    }
}
pub fn snapshot(root: &Path) -> Result<Snapshot> {
    let mut report = scan::scan()?;
    let saved = workspace::save_new(root, "Report.json", &report.hardware)?;
    let acpi = saved.parent().context("Scan session")?.join("ACPI");
    let mut notices = vec![];
    let acpi = match capture_acpi(&acpi) {
        Ok(count) => {
            notices.push(format!("Сохранено ACPI tables: {count}"));
            Some(acpi)
        }
        Err(e) => {
            notices.push(format!("ACPI не готова: {e}. Импортируйте полный дамп или повторите capture с правами администратора."));
            None
        }
    };
    let parent = saved.parent().context("Scan session")?;
    fs::write(
        parent.join("capture.json"),
        serde_json::to_vec_pretty(&Snapshot {
            report: saved.clone(),
            acpi: acpi.clone(),
            notices: notices.clone(),
        })?,
    )?;
    // Preserve the report bytes on disk as the authoritative source for its digest.
    report = Report::load(&saved)?;
    let _ = report;
    Ok(Snapshot {
        report: saved,
        acpi,
        notices,
    })
}
