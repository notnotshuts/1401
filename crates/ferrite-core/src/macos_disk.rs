//! Disk Arbitration ownership remains alive during every write and verification read.
use crate::{disks::Disk, macos, usb};
use anyhow::{Context, Result, bail};
use plist::Value;
use std::{
    ffi::{CStr, CString, c_char, c_int, c_void},
    fs::{File, OpenOptions},
    io::Cursor,
    os::{
        fd::AsRawFd,
        unix::fs::{MetadataExt, OpenOptionsExt},
    },
    path::Path,
    ptr::NonNull,
    time::Duration,
};

unsafe extern "C" {
    fn ferrite_media_identity(
        bsd: *const c_char,
        id: *mut u64,
        usb: *mut c_int,
        serial: *mut c_char,
        capacity: usize,
    ) -> c_int;
    fn ferrite_claim_unmount(bsd: *const c_char, expected: u64, error: *mut c_int) -> *mut c_void;
    fn ferrite_release(lease: *mut c_void);
    fn ferrite_no_mounts(bsd: *const c_char) -> c_int;
    fn ferrite_geometry(fd: c_int, bytes: *mut u64, sector: *mut u32) -> c_int;
    fn ferrite_flush(fd: c_int) -> c_int;
}

fn native_result(code: c_int, operation: &str) -> Result<()> {
    if code != 0 {
        bail!(
            "{operation}: native status {code}. Закройте окна и процессы, использующие флешку; принудительное размонтирование не выполняется."
        );
    }
    Ok(())
}

pub(crate) fn sync_device(file: &File) -> Result<()> {
    native_result(
        unsafe { ferrite_flush(file.as_raw_fd()) },
        "Block buffers/hardware cache flush",
    )
}

pub fn identity(id: &str) -> Result<macos::MediaIdentity> {
    if !macos::valid_disk_id(id) {
        bail!("Expected a whole disk identifier");
    }
    let bsd = CString::new(id)?;
    let mut registry_id = 0;
    let mut usb = 0;
    let mut serial = [0 as c_char; 1024];
    // SAFETY: buffers and CString remain alive for the synchronous native call.
    native_result(
        unsafe {
            ferrite_media_identity(
                bsd.as_ptr(),
                &mut registry_id,
                &mut usb,
                serial.as_mut_ptr(),
                serial.len(),
            )
        },
        "IOKit identity",
    )?;
    Ok(macos::MediaIdentity {
        registry_id,
        usb: usb != 0,
        serial: unsafe { CStr::from_ptr(serial.as_ptr()) }
            .to_string_lossy()
            .trim()
            .to_string(),
    })
}

pub fn info(id: &str) -> Result<Value> {
    if !macos::valid_disk_id(id) {
        bail!("Expected a whole disk identifier");
    }
    let raw = crate::workspace::run_bounded(
        "/usr/sbin/diskutil",
        &["info", "-plist", id],
        Duration::from_secs(30),
    )?;
    Ok(Value::from_reader(Cursor::new(raw))?)
}

pub fn describe(id: &str) -> Result<Disk> {
    crate::disks::list()?
        .into_iter()
        .find(|d| d.path == format!("/dev/{id}"))
        .context("Physical USB disappeared from inventory")
}

struct Lease(NonNull<c_void>);
impl Drop for Lease {
    fn drop(&mut self) {
        // SAFETY: this guard uniquely owns the native allocation; C drains its callback queue before freeing it.
        unsafe {
            ferrite_release(self.0.as_ptr());
        }
    }
}

pub struct Guard {
    file: Option<File>,
    lease: Option<Lease>,
    bsd: CString,
    selected: Disk,
}

impl Guard {
    pub fn acquire(selected: &Disk) -> Result<Self> {
        if unsafe { libc::geteuid() } != 0 {
            bail!("macOS USB writer requires the authorized CLI helper");
        }
        let id = macos::disk_path_id(&selected.path)?;
        let current = describe(id)?;
        usb::same_disk(selected, &current)?;
        if !current.eligible() {
            bail!("Стирание запрещено: {}", current.reasons.join("; "));
        }
        let guard = Self::open(selected)?;
        guard.recheck()?;
        Ok(guard)
    }

    fn open(selected: &Disk) -> Result<Self> {
        let id = macos::disk_path_id(&selected.path)?;
        let observed = identity(id)?;
        if format!("ioreg:{}", observed.registry_id) != selected.device_id {
            bail!("IOKit identity changed");
        }
        let bsd = CString::new(id)?;
        let mut error = 0;
        // SAFETY: native ownership is returned only after claim and whole-disk unmount complete.
        let lease =
            unsafe { ferrite_claim_unmount(bsd.as_ptr(), observed.registry_id, &mut error) };
        let lease = Lease(NonNull::new(lease).context(format!(
            "Disk Arbitration claim/unmount failed ({error}); диск не стёрт"
        ))?);
        native_result(
            unsafe { ferrite_no_mounts(bsd.as_ptr()) },
            "Volumes still mounted",
        )?;
        if std::fs::canonicalize(&selected.path)
            .context("Whole disk node disappeared after unmount")?
            != Path::new(&selected.path)
        {
            bail!("Disk symlinks are not accepted");
        }
        // Use the buffered block node: FAT directory and allocation-table writes are not sector aligned.
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&selected.path)
            .context("Open owned whole-disk block node")?;
        let mut bytes = 0;
        let mut sector = 0;
        native_result(
            unsafe { ferrite_geometry(file.as_raw_fd(), &mut bytes, &mut sector) },
            "Block geometry",
        )?;
        if bytes != selected.size
            || sector != 512
            || file.metadata()?.rdev() != std::fs::metadata(&selected.path)?.rdev()
        {
            bail!("Disk geometry or kernel device changed");
        }
        if identity(id)?.registry_id != observed.registry_id {
            bail!("Disk was replaced while opening");
        }
        Ok(Self {
            file: Some(file),
            lease: Some(lease),
            bsd,
            selected: selected.clone(),
        })
    }

    pub fn recheck(&self) -> Result<()> {
        let id = macos::disk_path_id(&self.selected.path)?;
        let current = describe(id)?;
        usb::same_disk(&self.selected, &current)?;
        if !current.eligible() {
            bail!("USB state changed: {}", current.reasons.join("; "));
        }
        native_result(
            unsafe { ferrite_no_mounts(self.bsd.as_ptr()) },
            "Volumes remounted",
        )
    }

    pub fn file(&mut self) -> &mut File {
        self.file.as_mut().expect("owned file")
    }

    pub fn finish(mut self) -> Result<Option<String>> {
        let file = self.file.as_ref().context("Disk closed")?;
        sync_device(file)?;
        self.file.take();
        self.lease.take();
        let eject = crate::workspace::run_bounded(
            "/usr/sbin/diskutil",
            &["eject", &self.selected.path],
            Duration::from_secs(30),
        );
        Ok(eject.err().map(|e| {
            format!("Файлы записаны и проверены; безопасное извлечение требует внимания: {e}")
        }))
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        // Close the block descriptor before releasing mount protection, including on errors.
        self.file.take();
        self.lease.take();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    struct Image {
        path: std::path::PathBuf,
        device: Option<String>,
    }
    impl Drop for Image {
        fn drop(&mut self) {
            if let Some(device) = &self.device {
                let _ = crate::workspace::run_bounded(
                    "/usr/bin/hdiutil",
                    &["detach", device],
                    Duration::from_secs(30),
                );
            }
        }
    }
    struct ReadOnlyMount(tempfile::TempDir);
    impl Drop for ReadOnlyMount {
        fn drop(&mut self) {
            let _ = crate::workspace::run_bounded(
                "/sbin/umount",
                &[self.0.path().to_str().unwrap_or("")],
                Duration::from_secs(30),
            );
        }
    }
    struct HeldProcess(std::process::Child);
    impl Drop for HeldProcess {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    impl Image {
        fn attach(&mut self, nomount: bool) -> Result<(String, Option<String>)> {
            let mut args = vec!["attach", "-plist", "-nobrowse"];
            if nomount {
                args.push("-nomount");
            }
            args.push(self.path.to_str().context("Image path")?);
            let raw =
                crate::workspace::run_bounded("/usr/bin/hdiutil", &args, Duration::from_secs(60))?;
            let data = Value::from_reader(Cursor::new(raw))?;
            let rows = data
                .as_dictionary()
                .and_then(|d| d.get("system-entities"))
                .and_then(Value::as_array)
                .context("hdiutil attach entities")?;
            let mut whole = None;
            let mut mount = None;
            for row in rows {
                let row = row.as_dictionary().context("Attached entity")?;
                if let Some(device) = row.get("dev-entry").and_then(Value::as_string)
                    && macos::disk_path_id(device).is_ok()
                {
                    whole = Some(device.to_string());
                }
                if let Some(path) = row.get("mount-point").and_then(Value::as_string) {
                    mount = Some(path.to_string());
                }
            }
            let device = whole.context("Whole virtual disk")?;
            self.device = Some(device.clone());
            Ok((device, mount))
        }
    }

    fn mount_owned_partition(device: &str) -> Result<String> {
        let partition = format!("{device}s1");
        crate::workspace::run_bounded(
            "/usr/sbin/diskutil",
            &["mount", &partition],
            Duration::from_secs(60),
        )
        .context("Explicitly mount owned test partition")?;
        let raw = crate::workspace::run_bounded(
            "/usr/sbin/diskutil",
            &["info", "-plist", &partition],
            Duration::from_secs(30),
        )?;
        let value = Value::from_reader(Cursor::new(raw))?;
        let mount = value
            .as_dictionary()
            .and_then(|p| p.get("MountPoint"))
            .and_then(Value::as_string)
            .filter(|p| !p.is_empty() && Path::new(p).is_dir())
            .context("Owned partition has no usable mount directory")?;
        Ok(mount.to_string())
    }

    #[test]
    #[ignore = "Root-only integration on an owned disposable GitHub Actions disk image"]
    fn virtual_disk_claim_and_write() -> Result<()> {
        if std::env::var("GITHUB_ACTIONS").as_deref() != Ok("true")
            || unsafe { libc::geteuid() } != 0
        {
            bail!("Only the root GitHub Actions integration runner may invoke this test");
        }
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("ferrite-ci.sparseimage");
        crate::workspace::run_bounded(
            "/usr/bin/hdiutil",
            &[
                "create",
                "-size",
                "8g",
                "-type",
                "SPARSE",
                "-layout",
                "NONE",
                path.to_str().context("Image path")?,
            ],
            Duration::from_secs(90),
        )?;
        println!("Created disposable image: {}", path.display());
        let mut image = Image { path, device: None };
        let (device, _) = image.attach(true).context("Attach empty owned image")?;
        println!("Attached owned image: {device}");
        let id = macos::disk_path_id(&device)?;
        let live = identity(id).context("Identify owned virtual disk")?;
        assert!(!live.usb);
        let metadata = info(id)?;
        let d = metadata.as_dictionary().context("Virtual metadata")?;
        assert_eq!(
            d.get("BusProtocol").and_then(Value::as_string),
            Some("Disk Image")
        );
        // Attest the backing file before invoking the private test route. No production flag permits images.
        let raw = crate::workspace::run_bounded(
            "/usr/bin/hdiutil",
            &["info", "-plist"],
            Duration::from_secs(30),
        )?;
        let all = Value::from_reader(Cursor::new(raw))?;
        let images = all
            .as_dictionary()
            .and_then(|x| x.get("images"))
            .and_then(Value::as_array)
            .context("Image inventory")?;
        assert!(images.iter().any(|row| {
            let Some(row) = row.as_dictionary() else {
                return false;
            };
            let owned = row
                .get("image-path")
                .and_then(Value::as_string)
                .and_then(|p| fs::canonicalize(p).ok())
                == fs::canonicalize(&image.path).ok();
            let matches = row
                .get("system-entities")
                .and_then(Value::as_array)
                .is_some_and(|entities| {
                    entities.iter().any(|e| {
                        e.as_dictionary()
                            .and_then(|e| e.get("dev-entry"))
                            .and_then(Value::as_string)
                            == Some(&device)
                    })
                });
            owned && matches
        }));
        crate::workspace::run_bounded(
            "/usr/sbin/diskutil",
            &["eraseDisk", "HFS+", "FERRITE-CI", "MBRFormat", &device],
            Duration::from_secs(90),
        )?;
        let live = identity(id).context("Identify owned virtual disk")?;
        let metadata = info(id)?;
        let d = metadata.as_dictionary().context("Formatted owned image")?;
        let initial_mount = mount_owned_partition(&device)?;
        println!("Formatted and mounted initial HFS+ owned image: {device} at {initial_mount}");
        let selected = Disk {
            path: device.clone(),
            model: "Owned CI image".into(),
            serial: format!("connection:{}", live.registry_id),
            size: d
                .get("TotalSize")
                .and_then(Value::as_unsigned_integer)
                .context("Image size")?,
            bus: "Disk Image".into(),
            reasons: vec!["Not physical USB".into()],
            device_id: format!("ioreg:{}", live.registry_id),
            sector_size: 512,
        };
        assert!(
            Guard::acquire(&selected).is_err(),
            "production must refuse a disk image"
        );
        let mut stale = selected.clone();
        stale.device_id = "ioreg:0".into();
        assert!(
            Guard::open(&stale).is_err(),
            "changed identity must stop before claiming"
        );
        // A process with its working directory on the volume must block non-forced unmount.
        {
            use std::io::BufRead;
            let mount = initial_mount;
            println!("Testing busy volume at {mount}");
            let child = std::process::Command::new("/bin/sh")
                .args(["-c", "pwd; exec sleep 120"])
                .current_dir(&mount)
                .stdout(std::process::Stdio::piped())
                .spawn()
                .context("Start busy-volume process in initial mount")?;
            let mut held = HeldProcess(child);
            let stdout = held.0.stdout.take().context("Busy process output")?;
            let mut ready = String::new();
            std::io::BufReader::new(stdout).read_line(&mut ready)?;
            assert!(!ready.is_empty());
            assert!(
                Guard::open(&selected).is_err(),
                "a busy volume must not be force-unmounted"
            );
        }
        println!("Acquiring unoccupied owned disk");
        let mut guard =
            Guard::open(&selected).context("Claim and open owned disk after busy-volume test")?;
        assert!(
            Guard::open(&selected).is_err(),
            "a second client must not claim the disk"
        );
        // mountDisk reports aggregate success even when an individual volume is denied.
        let attempt = crate::workspace::run_bounded(
            "/usr/sbin/diskutil",
            &["mount", &format!("{device}s1")],
            Duration::from_secs(20),
        );
        println!("Mount attempt during lease: {attempt:?}");
        native_result(
            unsafe { ferrite_no_mounts(guard.bsd.as_ptr()) },
            "Mount protection failed",
        )?;
        let payload = temp.path().join("efi-fixture");
        fs::write(&payload, b"Ferrite native USB integration fixture")?;
        let result = usb::test_macos_payload(guard.file(), selected.size, &payload)?;
        assert!(result.verified);
        assert!(
            guard.finish()?.is_none(),
            "Eject must succeed on the owned image"
        );
        image.device = None;
        let (device, _) = image.attach(true)?;
        crate::workspace::run_bounded(
            "/sbin/fsck_msdos",
            &["-n", &format!("{device}s1")],
            Duration::from_secs(30),
        )
        .context("Written FAT32 must pass Apple's filesystem checker")?;
        let mut native_mount = None;
        let mount = match mount_owned_partition(&device) {
            Ok(mount) => mount,
            Err(error) => {
                println!(
                    "Disk Utility FAT mount failed after clean fsck: {error:#}; checking native read-only mount"
                );
                let directory = tempfile::tempdir_in(temp.path())?;
                crate::workspace::run_bounded(
                    "/sbin/mount_msdos",
                    &[
                        "-o",
                        "rdonly,nobrowse",
                        &format!("{device}s1"),
                        directory.path().to_str().context("Owned mount directory")?,
                    ],
                    Duration::from_secs(30),
                )
                .context("Native read-only FAT mount after Disk Utility failure")?;
                let path = directory.path().to_string_lossy().into_owned();
                native_mount = Some(ReadOnlyMount(directory));
                path
            }
        };
        assert_eq!(
            fs::read(Path::new(&mount).join("EFI/BOOT/BOOTx64.efi"))?,
            fs::read(&payload)?
        );
        drop(native_mount);
        println!(
            "Native Disk Arbitration claim, mount denial, buffered-device FAT32 write, clean Apple fsck, checksum read-back, eject and reattachment passed."
        );
        Ok(())
    }
}
