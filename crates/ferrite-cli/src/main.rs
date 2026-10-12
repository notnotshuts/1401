use anyhow::Result;
use clap::{Parser, Subcommand};
use ferrite_core::{
    builder, capture, compatibility, disks, efi, planner, recovery, report::Report, scan, usb,
    workspace,
};
use std::{path::PathBuf, process::ExitCode};
#[derive(Parser)]
#[command(
    name = "ferrite",
    version,
    about = "Ferrite — аппаратная диагностика и проверка OpenCore для Windows/Linux/macOS"
)]
struct Args {
    /// Language for the installation guide
    #[arg(long, global = true, default_value = "ru", value_parser = ["ru", "en", "es"])]
    language: String,
    #[command(subcommand)]
    command: Action,
}
#[derive(Subcommand)]
enum Action {
    /// Save a step-by-step installation guide in the selected language
    Guide { output: PathBuf },
    /// Сканирование аппаратуры и попытка снять ACPI в новую сессию
    Snapshot { output: PathBuf },
    /// Снять ACPI без изменения оборудования
    Capture { output: PathBuf },
    /// Узнать метаданные Apple Recovery (без публикации временных токенов)
    RecoveryInfo {
        #[arg(long, default_value_t = 24)]
        darwin: u8,
    },
    /// Загрузить и проверить подписанный Apple Recovery
    Recovery {
        output: PathBuf,
        #[arg(long, default_value_t = 24)]
        darwin: u8,
    },
    /// Создать новый проверенный образ установочного USB
    Image {
        build: PathBuf,
        recovery: PathBuf,
        output: PathBuf,
        #[arg(long, default_value_t = 8)]
        size_gib: u64,
    },
    /// СТЕРЕТЬ выбранный USB после проверки identity, EFI и подписанного Recovery
    Write {
        build: PathBuf,
        recovery: PathBuf,
        #[arg(long)]
        selection: PathBuf,
        #[arg(long)]
        confirm: String,
        #[arg(long)]
        result: Option<PathBuf>,
    },
    /// Спланировать OpenCore, kexts, SMBIOS и ограничения
    Plan {
        report: PathBuf,
        #[arg(long, default_value_t = 24)]
        darwin: u8,
    },
    /// Собрать EFI из закреплённых компонентов и проверить ocvalidate
    Build {
        report: PathBuf,
        acpi: PathBuf,
        output: PathBuf,
        #[arg(long, default_value_t = 24)]
        darwin: u8,
        #[arg(long, default_value = "iasl")]
        iasl: String,
        #[arg(long)]
        cache: Option<PathBuf>,
        #[arg(long)]
        smbios: Option<String>,
        #[arg(long)]
        audio_layout: Option<u64>,
    },
    /// Сканировать этот ПК без изменения оборудования
    Scan {
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Оценить совместимость импортированного Hardware Sniffer Report.json
    Check {
        report: PathBuf,
        #[arg(long, default_value_t = 24)]
        darwin: u8,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Показать накопители и причины блокировки
    Disks,
    /// Проверить существующую EFI; без ocvalidate результат только структурный
    Audit {
        efi: PathBuf,
        #[arg(long, requires = "validator_sha256")]
        validator: Option<PathBuf>,
        #[arg(long, requires = "validator")]
        validator_sha256: Option<String>,
    },
    /// Сохранить отдельную копию EFI с исправлением дубликатов и NVRAM/Delete
    Repair { efi: PathBuf, output: PathBuf },
}
fn execute() -> Result<bool> {
    let args = Args::parse();
    ferrite_core::i18n::set_language(ferrite_core::i18n::Language::from_locale(&args.language));
    match args.command {
        Action::Guide { output } => {
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&output)?;
            file.write_all(
                ferrite_core::guide::render(None, ferrite_core::i18n::language()).as_bytes(),
            )?;
            file.sync_all()?;
            println!("{}", output.display());
            Ok(true)
        }
        Action::Snapshot { output } => {
            println!(
                "{}",
                serde_json::to_string_pretty(&capture::snapshot(&output)?)?
            );
            Ok(true)
        }
        Action::Capture { output } => {
            println!("ACPI tables: {}", capture::capture_acpi(&output)?);
            Ok(true)
        }
        Action::RecoveryInfo { darwin } => {
            println!(
                "{}",
                serde_json::to_string_pretty(&recovery::info(darwin)?)?
            );
            Ok(true)
        }
        Action::Recovery { output, darwin } => {
            let info = recovery::info(darwin)?;
            let result = recovery::download(&info, &output, &mut |name, done, total| {
                eprintln!("{name}: {done}/{total}")
            })?;
            println!("{}", serde_json::to_string_pretty(&result)?);
            Ok(true)
        }
        Action::Image {
            build,
            recovery,
            output,
            size_gib,
        } => {
            let r = usb::image(
                &build,
                &recovery,
                &output,
                size_gib,
                &mut |name, done, total| {
                    if done == total {
                        eprintln!("{name}: {done}/{total}");
                    }
                },
            )?;
            println!("{}", serde_json::to_string_pretty(&r)?);
            Ok(r.verified)
        }
        Action::Write {
            build,
            recovery,
            selection,
            confirm,
            result,
        } => {
            let d: disks::Disk = serde_json::from_slice(&std::fs::read(selection)?)?;
            let r = usb::disk(&d, &confirm, &build, &recovery, &mut |name, done, total| {
                if done == total {
                    eprintln!("{name}: {done}/{total}");
                }
            })?;
            let json = serde_json::to_vec_pretty(&r)?;
            if let Some(path) = result {
                std::fs::write(path, &json)?;
            }
            println!("{}", String::from_utf8(json)?);
            Ok(r.verified)
        }
        Action::Plan { report, darwin } => {
            let result = planner::plan(
                &Report::load(&report)?,
                &planner::Options {
                    target: darwin,
                    ..Default::default()
                },
            )?;
            println!("{}", serde_json::to_string_pretty(&result)?);
            Ok(true)
        }
        Action::Build {
            report,
            acpi,
            output,
            darwin,
            iasl,
            cache,
            smbios,
            audio_layout,
        } => {
            let result = builder::build(
                &report,
                &acpi,
                &output,
                &cache.unwrap_or_else(|| workspace::default_workspace().join("cache")),
                &iasl,
                &planner::Options {
                    target: darwin,
                    smbios,
                    audio_layout,
                    ..Default::default()
                },
                &mut |name, done, total| {
                    if done == 0 || done == total {
                        eprintln!("{name}: {done}/{total}");
                    }
                },
            )?;
            println!("{}", serde_json::to_string_pretty(&result)?);
            Ok(result.ok)
        }
        Action::Scan { output } => {
            let report = scan::scan()?;
            let root = output.unwrap_or_else(workspace::default_workspace);
            let path = workspace::save_new(&root, "Report.json", &report.hardware)?;
            println!("{}", path.display());
            Ok(true)
        }
        Action::Check {
            report,
            darwin,
            output,
        } => {
            let result = compatibility::analyze(&Report::load(&report)?, darwin);
            if let Some(root) = output {
                let p = workspace::save_new(&root, "compatibility.json", &result)?;
                eprintln!("{}", p.display());
            }
            println!("{}", serde_json::to_string_pretty(&result)?);
            Ok(result.ok)
        }
        Action::Disks => {
            println!("{}", serde_json::to_string_pretty(&disks::list()?)?);
            Ok(true)
        }
        Action::Audit {
            efi,
            validator,
            validator_sha256,
        } => {
            let result = efi::audit(&efi, validator.as_deref().zip(validator_sha256.as_deref()))?;
            println!("{}", serde_json::to_string_pretty(&result)?);
            Ok(result.ready)
        }
        Action::Repair { efi, output } => {
            let changes = efi::repair_copy(&efi, &output)?;
            println!("{}", serde_json::to_string_pretty(&changes)?);
            eprintln!(
                "Копия: {}. Требуется повторная проверка ocvalidate.",
                output.display()
            );
            Ok(true)
        }
    }
}
fn main() -> ExitCode {
    match execute() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(2),
        Err(e) => {
            eprintln!("Ferrite: {e:#}");
            ExitCode::FAILURE
        }
    }
}
