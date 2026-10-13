pub mod compatibility;
pub mod disks;
pub mod efi;
pub mod report;
pub mod scan;
pub mod workspace;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const RULES_REVISION: &str = "opcore-e5d8a9f-1401-2026-10-12";

pub mod acpi;
pub mod builder;
pub mod capture;
pub mod config;
pub mod cpu;
pub mod downloads;
pub mod elevation;
pub mod planner;
pub mod recovery;
pub mod usb;
#[cfg(target_os = "windows")]
pub mod windows_disk;
#[cfg(target_os = "windows")]
pub mod windows_scan;

pub mod i18n;

pub mod guide;

pub mod macos;
pub mod platform;

#[cfg(target_os = "macos")]
pub mod macos_disk;
