#[cfg(target_os = "linux")]
use anyhow::Context;
use anyhow::Result;
use serde::{Deserialize, Serialize};
#[cfg(target_os = "linux")]
use serde_json::Value;
#[cfg(target_os = "linux")]
use std::time::Duration;
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Disk {
    pub path: String,
    pub model: String,
    pub serial: String,
    pub size: u64,
    pub bus: String,
    pub reasons: Vec<String>,
    #[serde(default)]
    pub device_id: String,
    #[serde(default = "default_sector")]
    pub sector_size: u32,
}
fn default_sector() -> u32 {
    512
}
impl Disk {
    pub fn eligible(&self) -> bool {
        self.reasons.is_empty()
    }
}
pub fn list() -> Result<Vec<Disk>> {
    #[cfg(target_os = "linux")]
    {
        let raw = crate::workspace::run_bounded(
            "lsblk",
            &[
                "--json",
                "--tree",
                "--bytes",
                "--paths",
                "--output",
                "PATH,TYPE,SIZE,MODEL,SERIAL,TRAN,RO,FSTYPE,PARTTYPE,MOUNTPOINTS,MAJ:MIN,LOG-SEC",
            ],
            Duration::from_secs(20),
        )?;
        let data: Value = serde_json::from_slice(&raw)?;
        Ok(data["blockdevices"]
            .as_array()
            .context("lsblk: blockdevices отсутствует")?
            .iter()
            .filter(|n| n["type"] == "disk")
            .map(describe_linux)
            .collect())
    }
    #[cfg(target_os = "windows")]
    {
        crate::windows_disk::list()
    }
    #[cfg(target_os = "macos")]
    {
        crate::macos::list_disks()
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
    {
        anyhow::bail!("Диски поддерживаются только на Windows/Linux")
    }
}
#[cfg(target_os = "linux")]
fn walk<'a>(node: &'a Value, out: &mut Vec<&'a Value>) {
    out.push(node);
    if let Some(children) = node["children"].as_array() {
        for c in children {
            walk(c, out);
        }
    }
}
#[cfg(target_os = "linux")]
pub fn describe_linux(node: &Value) -> Disk {
    let s = |key: &str| node[key].as_str().unwrap_or("").trim().to_string();
    let mut reasons = vec![];
    let bus = s("tran");
    let serial = s("serial");
    let size = node["size"].as_u64().unwrap_or(0);
    if bus != "usb" {
        reasons.push("Не USB-накопитель".into());
    }
    if serial.is_empty() {
        reasons.push("Нет стабильного серийного номера".into());
    }
    if size < 8 * 1024 * 1024 * 1024 {
        reasons.push("Размер меньше 8 GiB".into());
    }
    if size > 128 * 1024 * 1024 * 1024 {
        reasons.push("Большой накопитель: требуется отдельная проверка".into());
    }
    let mut nodes = vec![];
    walk(node, &mut nodes);
    for n in nodes {
        if n["ro"].as_bool() != Some(false) {
            reasons.push("Накопитель read-only или состояние неизвестно".into());
        }
        if !matches!(n["type"].as_str(), Some("disk" | "part")) {
            reasons.push("Занятое устройство mapper/RAID".into());
        }
        let fs = n["fstype"].as_str().unwrap_or("");
        let part = n["parttype"].as_str().unwrap_or("").to_lowercase();
        if [
            "ext2",
            "ext3",
            "ext4",
            "btrfs",
            "xfs",
            "zfs_member",
            "linux_raid_member",
            "LVM2_member",
            "crypto_LUKS",
            "swap",
            "apfs",
            "hfs",
            "hfsplus",
            "ntfs",
            "BitLocker",
        ]
        .contains(&fs)
            || [
                "c12a7328-f81f-11d2-ba4b-00a0c93ec93b",
                "7c3457ef-0000-11aa-aa11-00306543ecac",
                "48465300-0000-11aa-aa11-00306543ecac",
                "e3c9e316-0b5c-4db8-817d-f92df00215ae",
                "de94bba4-06d1-4d40-a16a-bfd50179d6ac",
                "0x82",
                "0x83",
                "0x8e",
            ]
            .contains(&part.as_str())
        {
            reasons.push("Обнаружены разделы установленной ОС".into());
        }
        if let Some(mounts) = n["mountpoints"].as_array() {
            for m in mounts {
                if let Some(m) = m.as_str()
                    && !(m.starts_with("/media/")
                        || m.starts_with("/run/media/")
                        || m.starts_with("/mnt/"))
                {
                    reasons.push(format!("Защищённая точка монтирования: {m}"));
                }
            }
        } else {
            reasons.push("Неизвестны точки монтирования".into());
        }
    }
    reasons.sort();
    reasons.dedup();
    Disk {
        path: s("path"),
        model: s("model"),
        serial,
        size,
        bus,
        reasons,
        device_id: s("maj:min"),
        sector_size: node["log-sec"].as_u64().unwrap_or(512) as u32,
    }
}
#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn detects_nested_system_partition() {
        let disk = describe_linux(
            &json!({"path":"/dev/sdb","type":"disk","size":16000000000u64,"serial":"123","tran":"usb","ro":false,"mountpoints":[],"children":[{"type":"part","ro":false,"fstype":"ext4","mountpoints":["/"]}]}),
        );
        assert!(!disk.eligible());
        assert!(disk.reasons.iter().any(|x| x.contains("ОС")));
    }
    #[test]
    fn rejects_missing_identity() {
        assert!(!describe_linux(&json!({"tran":"usb","type":"disk","size":16000000000u64,"ro":false,"mountpoints":[]})).eligible());
    }
}
