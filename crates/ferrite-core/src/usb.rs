//! Shared FAT32/MBR writer, guarded by platform-native disk identity and locks.
#[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
use crate::disks;
use crate::{builder::Build, disks::Disk, downloads, recovery};
use anyhow::{Context, Result, bail};
use fatfs::{FatType, FileSystem, FormatVolumeOptions, FsOptions};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
#[cfg(target_os = "linux")]
use std::time::Duration;
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Component, Path},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Written {
    pub device: String,
    pub bytes: u64,
    pub files: usize,
    pub verified: bool,
    pub notices: Vec<String>,
}
pub fn same_disk(selected: &Disk, current: &Disk) -> Result<()> {
    if selected.path != current.path
        || selected.serial.is_empty()
        || selected.serial != current.serial
        || selected.size != current.size
        || selected.model != current.model
        || selected.bus != current.bus
        || selected.device_id != current.device_id
        || selected.sector_size != current.sector_size
    {
        bail!("Накопитель изменился после выбора; стирание запрещено");
    }
    Ok(())
}
struct Partition<'a> {
    file: &'a mut File,
    start: u64,
    len: u64,
    position: u64,
}
impl Read for Partition<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let count = (self.len - self.position).min(buf.len() as u64) as usize;
        self.file
            .seek(SeekFrom::Start(self.start + self.position))?;
        let n = self.file.read(&mut buf[..count])?;
        self.position += n as u64;
        Ok(n)
    }
}
impl Write for Partition<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let count = (self.len - self.position).min(buf.len() as u64) as usize;
        if count == 0 && !buf.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::WriteZero,
                "FAT partition boundary",
            ));
        }
        self.file
            .seek(SeekFrom::Start(self.start + self.position))?;
        let n = self.file.write(&buf[..count])?;
        self.position += n as u64;
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }
}
impl Seek for Partition<'_> {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        let at = match pos {
            SeekFrom::Start(n) => n as i128,
            SeekFrom::Current(n) => self.position as i128 + n as i128,
            SeekFrom::End(n) => self.len as i128 + n as i128,
        };
        if at < 0 || at > self.len as i128 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Seek outside FAT partition",
            ));
        }
        self.position = at as u64;
        Ok(self.position)
    }
}
#[derive(Clone)]
struct Payload {
    path: String,
    source: std::path::PathBuf,
    size: u64,
    sha: String,
}
fn checked_relative(path: &str) -> Result<()> {
    if path.is_empty()
        || path.contains(['\\', ':'])
        || Path::new(path)
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        bail!("Небезопасный путь manifest: {path}");
    }
    Ok(())
}
fn payloads(build_root: &Path, recovery_root: &Path) -> Result<(Vec<Payload>, tempfile::TempDir)> {
    let mut raw = Vec::new();
    File::open(build_root.join("build-result.json"))?
        .take(4 * 1024 * 1024 + 1)
        .read_to_end(&mut raw)?;
    if raw.len() > 4 * 1024 * 1024 {
        bail!("Build manifest слишком велик");
    }
    let build: Build = serde_json::from_slice(&raw)?;
    if !build.ok {
        bail!("EFI build не завершён");
    }
    // A serialized 'ready' flag is not sufficient: verify actual bytes and paths.
    let audit = crate::efi::audit(&build_root.join("EFI"), None)?;
    if !audit.issues.is_empty() {
        bail!("EFI изменена/повреждена: {}", audit.issues.join("; "));
    }
    let frozen = tempfile::tempdir()?;
    let mut result = vec![];
    for (relative, sha) in &build.files {
        checked_relative(relative)?;
        if !(relative.starts_with("EFI/") || relative == "BIOS.txt") {
            continue;
        }
        let source = build_root.join(relative);
        if source.is_symlink() || downloads::hash_file(&source)? != *sha {
            bail!("Файлы EFI не соответствуют build manifest: {relative}");
        }
        let size = fs::metadata(&source)?.len();
        if size > u32::MAX as u64 {
            bail!("Файл {relative} слишком велик для FAT32");
        }
        let frozen_source = frozen.path().join(relative);
        if let Some(parent) = frozen_source.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(&source, &frozen_source)?;
        if downloads::hash_file(&frozen_source)? != *sha {
            bail!("EFI изменилась во время фиксации файлов");
        }
        result.push(Payload {
            path: relative.clone(),
            source: frozen_source,
            size,
            sha: sha.clone(),
        });
    }
    if !result.iter().any(|p| p.path == "EFI/OC/config.plist")
        || !result.iter().any(|p| p.path == "EFI/BOOT/BOOTx64.efi")
    {
        bail!("Build manifest не содержит обязательную EFI");
    }
    let image = recovery_root.join("BaseSystem.dmg");
    let chunklist = recovery_root.join("BaseSystem.chunklist");
    recovery::verify(&image, &chunklist)?;
    for source in [image, chunklist] {
        let size = fs::metadata(&source)?.len();
        if size > u32::MAX as u64 {
            bail!("Recovery file превышает ограничение FAT32 4 GiB");
        }
        let name = source
            .file_name()
            .context("Recovery filename")?
            .to_string_lossy();
        result.push(Payload {
            path: format!("com.apple.recovery.boot/{name}"),
            sha: downloads::hash_file(&source)?,
            source,
            size,
        });
    }
    let validator_package = downloads::product("OpenCorePkg")?;
    let archive = frozen.path().join("OpenCorePackage.zip");
    downloads::snapshot_artifact(
        &build_root.join("OpenCorePackage.zip"),
        &validator_package,
        &archive,
    )?;
    let extracted = frozen.path().join("opencore");
    downloads::extract_zip(&archive, &extracted, &validator_package.sha256)?;
    let expected_name = crate::platform::utility_name("ocvalidate", crate::platform::host())?;
    let validator = downloads::files(&extracted, expected_name)?
        .into_iter()
        .find(|p| p.file_name().is_some_and(|n| n == expected_name))
        .context("ocvalidate отсутствует")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&validator, fs::Permissions::from_mode(0o755))?;
    }
    let trusted_sha = downloads::hash_file(&validator)?;
    let validation =
        crate::efi::audit(&frozen.path().join("EFI"), Some((&validator, &trusted_sha)))?;
    if !validation.ready {
        bail!(
            "Повторная проверка EFI перед записью не прошла: {}",
            validation.issues.join("; ")
        );
    }
    Ok((result, frozen))
}
fn write_payloads(
    file: &mut File,
    size: u64,
    payloads: &[Payload],
    progress: &mut impl FnMut(&str, u64, u64),
) -> Result<Written> {
    if !(8 * 1024 * 1024 * 1024..=128 * 1024 * 1024 * 1024).contains(&size)
        || !size.is_multiple_of(512)
    {
        bail!("Некорректный размер USB");
    }
    let start = 2048u32;
    let sectors = ((size / 512) - start as u64 - 2048).min(32 * 1024 * 1024) as u32;
    let part_size = sectors as u64 * 512;
    let total: u64 = payloads.iter().map(|p| p.size).sum();
    if total + 256 * 1024 * 1024 > part_size {
        bail!("Недостаточно места на разделе FAT32");
    }
    // GPT headers at both ends are removed only after all preflight checks and locks succeed.
    let zeros = vec![0u8; 1024 * 1024];
    file.seek(SeekFrom::Start(0))?;
    file.write_all(&zeros)?;
    file.seek(SeekFrom::Start(size - 1024 * 1024))?;
    file.write_all(&zeros)?;
    let mut mbr = [0u8; 512];
    mbr[446] = 0x80;
    mbr[447..450].copy_from_slice(&[0xfe, 0xff, 0xff]);
    mbr[450] = 0x0c;
    mbr[451..454].copy_from_slice(&[0xfe, 0xff, 0xff]);
    mbr[454..458].copy_from_slice(&start.to_le_bytes());
    mbr[458..462].copy_from_slice(&sectors.to_le_bytes());
    mbr[510] = 0x55;
    mbr[511] = 0xaa;
    file.seek(SeekFrom::Start(0))?;
    file.write_all(&mbr)?;
    file.sync_all()?;
    let mut partition = Partition {
        file,
        start: start as u64 * 512,
        len: part_size,
        position: 0,
    };
    fatfs::format_volume(
        &mut partition,
        FormatVolumeOptions::new()
            .fat_type(FatType::Fat32)
            .bytes_per_sector(512)
            .total_sectors(sectors)
            .volume_label(*b"FERRITE    "),
    )?;
    partition.seek(SeekFrom::Start(0))?;
    let volume = FileSystem::new(partition, FsOptions::new())?;
    let root = volume.root_dir();
    let mut done = 0;
    for payload in payloads {
        checked_relative(&payload.path)?;
        let (parent, name) = payload
            .path
            .rsplit_once('/')
            .unwrap_or(("", payload.path.as_str()));
        let mut directory = root.clone();
        for segment in parent.split('/').filter(|s| !s.is_empty()) {
            directory = directory.create_dir(segment)?;
        }
        let mut target = directory.create_file(name)?;
        let mut source = File::open(&payload.source)?;
        let mut hash = Sha256::new();
        let mut buffer = [0u8; 64 * 1024];
        let mut count = 0;
        loop {
            let n = source.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            count += n as u64;
            if count > payload.size {
                bail!("Исходный файл изменился во время записи");
            }
            target.write_all(&buffer[..n])?;
            hash.update(&buffer[..n]);
            done += n as u64;
            progress("Запись USB", done, total);
        }
        if count != payload.size || format!("{:x}", hash.finalize()) != payload.sha {
            bail!("Исходный файл изменился во время записи");
        }
        target.flush()?;
    }
    for payload in payloads {
        let mut target = root.open_file(&payload.path)?;
        let mut hash = Sha256::new();
        let mut buffer = [0u8; 64 * 1024];
        let mut count = 0;
        loop {
            let n = target.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            count += n as u64;
            hash.update(&buffer[..n]);
        }
        if count != payload.size || format!("{:x}", hash.finalize()) != payload.sha {
            bail!("Записанный файл не прошёл проверку: {}", payload.path);
        }
    }
    drop(root);
    volume.unmount()?;
    file.sync_all()?;
    Ok(Written {
        device: String::new(),
        bytes: total,
        files: payloads.len(),
        verified: true,
        notices: vec![],
    })
}
pub fn disk(
    selected: &Disk,
    confirm: &str,
    build: &Path,
    recovery: &Path,
    progress: &mut impl FnMut(&str, u64, u64),
) -> Result<Written> {
    #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
    {
        // Unsupported hosts must never validate payloads or open a physical disk.
        let _ = (selected, confirm, build, recovery, progress);
        bail!("USB writer is unsupported on this host");
    }
    #[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
    {
        disk_supported(selected, confirm, build, recovery, progress)
    }
}

#[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
fn disk_supported(
    selected: &Disk,
    confirm: &str,
    build: &Path,
    recovery: &Path,
    progress: &mut impl FnMut(&str, u64, u64),
) -> Result<Written> {
    if selected.sector_size != 512 {
        bail!("Для записи нужен логический сектор 512 байт");
    }
    if confirm != selected.path {
        bail!("Подтверждение должно точно повторять путь накопителя");
    }
    let (payloads, _frozen) = payloads(build, recovery)?;
    let current = disks::list()?
        .into_iter()
        .find(|d| d.path == selected.path)
        .context("Накопитель больше не подключён")?;
    same_disk(selected, &current)?;
    if !current.eligible() {
        bail!("Стирание запрещено: {}", current.reasons.join("; "));
    }
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt};
        if unsafe { libc::geteuid() } != 0 {
            bail!("Для записи требуется отдельный привилегированный helper");
        }
        if !selected.path.starts_with("/dev/")
            || fs::canonicalize(&selected.path)? != Path::new(&selected.path)
        {
            bail!("Нужен канонический whole-disk путь /dev");
        }
        // All descendants must have a known empty holder set. Dismount only ordinary removable-media mounts.
        let raw = crate::workspace::run_bounded(
            "lsblk",
            &[
                "--json",
                "--tree",
                "--paths",
                "--output",
                "PATH,MAJ:MIN,MOUNTPOINTS",
                &selected.path,
            ],
            Duration::from_secs(20),
        )?;
        let tree: serde_json::Value = serde_json::from_slice(&raw)?;
        fn nodes(v: &serde_json::Value, out: &mut Vec<serde_json::Value>) {
            out.push(v.clone());
            if let Some(c) = v["children"].as_array() {
                for n in c {
                    nodes(n, out);
                }
            }
        }
        let mut all = vec![];
        for n in tree["blockdevices"].as_array().context("lsblk root")? {
            nodes(n, &mut all);
        }
        for n in &all {
            let id = n["maj:min"]
                .as_str()
                .context("Неизвестен kernel device id")?;
            let holders = Path::new("/sys/dev/block").join(id).join("holders");
            if !holders.is_dir() || fs::read_dir(holders)?.next().is_some() {
                bail!("Накопитель занят device mapper/RAID или состояние неизвестно");
            }
            for mount in n["mountpoints"]
                .as_array()
                .context("Неизвестны mountpoints")?
            {
                if let Some(m) = mount.as_str() {
                    if !m.starts_with("/media/")
                        && !m.starts_with("/run/media/")
                        && !m.starts_with("/mnt/")
                    {
                        bail!("Защищённая точка монтирования");
                    }
                    crate::workspace::run_bounded("umount", &["--", m], Duration::from_secs(30))?;
                }
            }
        }
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_EXCL | libc::O_CLOEXEC)
            .open(&selected.path)
            .context("Не удалось эксклюзивно заблокировать USB; ничего не стёрто")?;
        let metadata = file.metadata()?;
        if !metadata.file_type().is_block_device() {
            bail!("Запись разрешена только на физический block device");
        }
        let device_id = format!(
            "{}:{}",
            libc::major(metadata.rdev()),
            libc::minor(metadata.rdev())
        );
        if device_id != selected.device_id {
            bail!("Kernel device ID изменился");
        }
        let fresh = disks::list()?
            .into_iter()
            .find(|d| d.path == selected.path)
            .context("USB исчез")?;
        same_disk(selected, &fresh)?;
        if !fresh.eligible() {
            bail!("Состояние USB изменилось");
        }
        let mut result = write_payloads(&mut file, selected.size, &payloads, progress)?;
        result.device = selected.path.clone();
        if unsafe { libc::ioctl(std::os::fd::AsRawFd::as_raw_fd(&file), 0x125f) } < 0 {
            result
                .notices
                .push("Файлы проверены; для обновления таблицы разделов переподключите USB".into());
        }
        Ok(result)
    }
    #[cfg(target_os = "macos")]
    {
        let mut guard = crate::macos_disk::Guard::acquire(selected)?;
        guard.recheck()?;
        let mut result = write_payloads(guard.file(), selected.size, &payloads, progress)?;
        result.device = selected.path.clone();
        match guard.finish()? {
            None => result
                .notices
                .push("USB безопасно извлечён. Для просмотра файлов подключите его снова.".into()),
            Some(message) => result.notices.push(message),
        }
        Ok(result)
    }
    #[cfg(target_os = "windows")]
    {
        let mut guard = crate::windows_disk::lock(selected)?;
        let mut result = write_payloads(&mut guard.file, selected.size, &payloads, progress)?;
        result.device = selected.path.clone();
        let refresh = guard.file.try_clone()?;
        drop(guard);
        if crate::windows_disk::refresh(&refresh).is_err() {
            result
                .notices
                .push("Файлы проверены; для обновления разделов переподключите USB".into());
        }
        Ok(result)
    }
}
/// Non-destructive integration route; only creates a fresh regular disk-image file.
pub fn image(
    build: &Path,
    recovery: &Path,
    output: &Path,
    size_gib: u64,
    progress: &mut impl FnMut(&str, u64, u64),
) -> Result<Written> {
    let (payloads, _frozen) = payloads(build, recovery)?;
    if !(8..=128).contains(&size_gib) {
        bail!("Размер образа должен быть 8–128 GiB");
    }
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(output)?;
    let size = size_gib * 1024 * 1024 * 1024;
    file.set_len(size)?;
    let mut result = write_payloads(&mut file, size, &payloads, progress)?;
    result.device = output.display().to_string();
    Ok(result)
}
#[cfg(test)]
#[cfg(target_os = "macos")]
pub(crate) fn test_macos_payload(file: &mut File, size: u64, source: &Path) -> Result<Written> {
    let data = fs::read(source)?;
    write_payloads(
        file,
        size,
        &[Payload {
            path: "EFI/BOOT/BOOTx64.efi".into(),
            source: source.into(),
            size: data.len() as u64,
            sha: format!("{:x}", Sha256::digest(&data)),
        }],
        &mut |_, _, _| {},
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn partition_cannot_escape() {
        let temp = tempfile::NamedTempFile::new().unwrap();
        temp.as_file().set_len(100).unwrap();
        let mut file = temp.reopen().unwrap();
        let mut p = Partition {
            file: &mut file,
            start: 20,
            len: 30,
            position: 0,
        };
        assert!(p.seek(SeekFrom::Start(31)).is_err());
        p.seek(SeekFrom::Start(29)).unwrap();
        assert_eq!(p.write(b"abc").unwrap(), 1);
        assert!(p.write(b"x").is_err());
    }
    #[test]
    fn path_traversal_rejected() {
        for path in ["../EFI", "/etc/passwd", "C:/secret", "EFI\\..\\x"] {
            assert!(checked_relative(path).is_err());
        }
    }
    #[test]
    fn fat32_round_trip_on_sparse_image() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("file");
        fs::write(&source, b"test payload").unwrap();
        let payload = Payload {
            path: "EFI/BOOT/BOOTx64.efi".into(),
            source,
            size: 12,
            sha: format!("{:x}", Sha256::digest(b"test payload")),
        };
        let mut file = tempfile::tempfile().unwrap();
        let size = 8u64 * 1024 * 1024 * 1024;
        file.set_len(size).unwrap();
        assert!(
            write_payloads(&mut file, size, &[payload], &mut |_, _, _| {})
                .unwrap()
                .verified
        );
        file.seek(SeekFrom::Start(510)).unwrap();
        let mut sig = [0; 2];
        file.read_exact(&mut sig).unwrap();
        assert_eq!(sig, [0x55, 0xaa]);
    }
}
