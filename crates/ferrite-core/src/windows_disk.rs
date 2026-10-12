#![cfg(target_os = "windows")]
//! Native disk inspection and volume locks; no diskpart or PowerShell erasure.
use crate::disks::Disk;
use anyhow::{Context, Result, bail};
use std::{
    fs::File,
    mem::{offset_of, size_of},
    os::windows::io::{AsRawHandle, FromRawHandle},
    ptr,
};
use windows_sys::Win32::{
    Foundation::*,
    Storage::FileSystem::*,
    System::{IO::DeviceIoControl, Ioctl::*, SystemInformation::GetWindowsDirectoryW},
};
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}
fn string(v: &[u16]) -> String {
    String::from_utf16_lossy(&v[..v.iter().position(|c| *c == 0).unwrap_or(v.len())])
}
fn open(path: &str, write: bool) -> Result<File> {
    let path = wide(path);
    let access = if write {
        GENERIC_READ | GENERIC_WRITE
    } else {
        0
    };
    // SAFETY: NUL-terminated path, null optional parameters, handle transferred to File once.
    let handle = unsafe {
        CreateFileW(
            path.as_ptr(),
            access,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            ptr::null(),
            OPEN_EXISTING,
            0,
            ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(unsafe { File::from_raw_handle(handle) })
}
fn ioctl(file: &File, code: u32, input: &[u8], output_size: usize) -> Result<Vec<u8>> {
    let mut bytes = vec![0u8; output_size];
    let mut returned = 0;
    // SAFETY: Live synchronous handle and buffers sized for the duration of the call.
    let ok = unsafe {
        DeviceIoControl(
            file.as_raw_handle(),
            code,
            if input.is_empty() {
                ptr::null()
            } else {
                input.as_ptr().cast()
            },
            input.len() as u32,
            if bytes.is_empty() {
                ptr::null_mut()
            } else {
                bytes.as_mut_ptr().cast()
            },
            bytes.len() as u32,
            &mut returned,
            ptr::null_mut(),
        )
    };
    if ok == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    if returned as usize > bytes.len() {
        bail!("Oversized DeviceIoControl result");
    }
    bytes.truncate(returned as usize);
    Ok(bytes)
}
fn record<T: Copy>(bytes: &[u8], at: usize) -> Result<T> {
    if at
        .checked_add(size_of::<T>())
        .is_none_or(|n| n > bytes.len())
    {
        bail!("Truncated native disk record");
    }
    // SAFETY: Checked bounds; native structures are Copy and may be unaligned.
    Ok(unsafe { ptr::read_unaligned(bytes.as_ptr().add(at).cast::<T>()) })
}
fn ascii(bytes: &[u8], offset: u32) -> Result<String> {
    if offset == 0 {
        return Ok(String::new());
    }
    let bytes = bytes
        .get(offset as usize..)
        .context("Storage string offset")?;
    let end = bytes
        .iter()
        .position(|b| *b == 0)
        .context("Unterminated storage string")?;
    Ok(String::from_utf8_lossy(&bytes[..end]).trim().to_string())
}
struct Volume {
    path: String,
    disks: Vec<u32>,
    fs: String,
    system: bool,
}
struct Search(HANDLE);
impl Drop for Search {
    fn drop(&mut self) {
        unsafe {
            FindVolumeClose(self.0);
        }
    }
}
fn volumes() -> Result<Vec<Volume>> {
    let mut windows = [0u16; 32768];
    let n = unsafe { GetWindowsDirectoryW(windows.as_mut_ptr(), windows.len() as u32) };
    if n == 0 || n as usize >= windows.len() {
        bail!("Cannot identify Windows system root");
    }
    let windows = string(&windows);
    let root = windows.get(..3).context("Windows root")?;
    let mut system_volume = [0u16; 1024];
    if unsafe {
        GetVolumeNameForVolumeMountPointW(wide(root).as_ptr(), system_volume.as_mut_ptr(), 1024)
    } == 0
    {
        bail!("Cannot identify Windows system volume");
    }
    let system_volume = string(&system_volume);
    let mut buffer = [0u16; 1024];
    let handle = unsafe { FindFirstVolumeW(buffer.as_mut_ptr(), 1024) };
    if handle == INVALID_HANDLE_VALUE {
        bail!("Cannot enumerate Windows volumes");
    }
    let _guard = Search(handle);
    let mut out = vec![];
    loop {
        let path = string(&buffer);
        let kind = unsafe { GetDriveTypeW(wide(&path).as_ptr()) };
        if matches!(kind, 2 | 3) {
            let f = open(path.trim_end_matches('\\'), false)?;
            let raw = ioctl(&f, IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS, &[], 65536)?;
            let count: u32 = record(&raw, 0)?;
            if count > 128 {
                bail!("Too many volume extents");
            }
            let mut disks = vec![];
            for i in 0..count as usize {
                let e: DISK_EXTENT = record(
                    &raw,
                    offset_of!(VOLUME_DISK_EXTENTS, Extents) + i * size_of::<DISK_EXTENT>(),
                )?;
                disks.push(e.DiskNumber);
            }
            let mut filesystem = [0u16; 64];
            let ok = unsafe {
                GetVolumeInformationW(
                    wide(&path).as_ptr(),
                    ptr::null_mut(),
                    0,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    ptr::null_mut(),
                    filesystem.as_mut_ptr(),
                    64,
                )
            };
            out.push(Volume {
                system: path.eq_ignore_ascii_case(&system_volume),
                path,
                disks,
                fs: if ok != 0 {
                    string(&filesystem)
                } else {
                    String::new()
                },
            });
        }
        if unsafe { FindNextVolumeW(handle, buffer.as_mut_ptr(), 1024) } == 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() != Some(ERROR_NO_MORE_FILES as i32) {
                return Err(error.into());
            }
            break;
        }
        if out.len() > 1024 {
            bail!("Too many Windows volumes");
        }
    }
    Ok(out)
}
fn describe(file: &File, number: u32, volumes: &[Volume]) -> Result<Disk> {
    let raw = ioctl(file, IOCTL_STORAGE_QUERY_PROPERTY, &[0; 12], 65536)?;
    let descriptor: STORAGE_DEVICE_DESCRIPTOR = record(&raw, 0)?;
    let size: i64 = record(&ioctl(file, IOCTL_DISK_GET_LENGTH_INFO, &[], 8)?, 0)?;
    if size <= 0 {
        bail!("Invalid disk length");
    }
    let geometry: DISK_GEOMETRY_EX = record(
        &ioctl(file, IOCTL_DISK_GET_DRIVE_GEOMETRY_EX, &[], 1024)?,
        0,
    )?;
    let sector = geometry.Geometry.BytesPerSector;
    let serial = ascii(&raw, descriptor.SerialNumberOffset)?;
    let model = format!(
        "{} {}",
        ascii(&raw, descriptor.VendorIdOffset)?,
        ascii(&raw, descriptor.ProductIdOffset)?
    )
    .trim()
    .to_string();
    let bus = if descriptor.BusType == BusTypeUsb {
        "usb".to_string()
    } else {
        format!("Windows bus {}", descriptor.BusType)
    };
    let mut reasons = vec![];
    if descriptor.BusType != BusTypeUsb {
        reasons.push("Не USB-накопитель".into());
    }
    if serial.is_empty() {
        reasons.push("Нет стабильного серийного номера".into());
    }
    if sector != 512 {
        reasons.push("Нужен логический сектор 512 байт".into());
    }
    if size < 8 * 1024 * 1024 * 1024 {
        reasons.push("Размер меньше 8 GiB".into());
    }
    if size > 128 * 1024 * 1024 * 1024 {
        reasons.push("Размер больше 128 GiB".into());
    }
    for v in volumes.iter().filter(|v| v.disks.contains(&number)) {
        if v.system {
            reasons.push("Системный том Windows".into());
        }
        if !matches!(v.fs.to_uppercase().as_str(), "FAT" | "FAT32" | "EXFAT") {
            reasons.push(format!("Том ОС или неизвестная файловая система: {}", v.fs));
        }
    }
    let layout = ioctl(file, IOCTL_DISK_GET_DRIVE_LAYOUT_EX, &[], 65536)?;
    let partition_count: u32 = record(&layout, 4)?;
    if partition_count > 128 {
        bail!("Too many disk partitions");
    }
    for i in 0..partition_count as usize {
        let p: PARTITION_INFORMATION_EX = record(
            &layout,
            offset_of!(DRIVE_LAYOUT_INFORMATION_EX, PartitionEntry)
                + i * size_of::<PARTITION_INFORMATION_EX>(),
        )?;
        if p.PartitionLength == 0 {
            continue;
        }
        if p.IsServicePartition {
            reasons.push("Служебный раздел ОС".into());
        }
        match p.PartitionStyle {
            PARTITION_STYLE_MBR => {
                let p = unsafe { p.Anonymous.Mbr };
                if !matches!(p.PartitionType, 0 | 0x01 | 0x04 | 0x06 | 0x0b | 0x0c | 0x0e) {
                    reasons.push(format!(
                        "Раздел ОС/неизвестный MBR type 0x{:02X}",
                        p.PartitionType
                    ));
                }
            }
            PARTITION_STYLE_GPT => {
                let p = unsafe { p.Anonymous.Gpt };
                let basic =
                    windows_sys::core::GUID::from_u128(0xebd0a0a2_b9e5_4433_87c0_68b6b72699c7);
                if (p.PartitionType.data1 != basic.data1
                    || p.PartitionType.data2 != basic.data2
                    || p.PartitionType.data3 != basic.data3
                    || p.PartitionType.data4 != basic.data4)
                    || !volumes.iter().any(|v| {
                        v.disks.contains(&number)
                            && matches!(v.fs.to_uppercase().as_str(), "FAT" | "FAT32" | "EXFAT")
                    })
                {
                    reasons.push("GPT: раздел ОС или непроверенный раздел".into());
                }
            }
            PARTITION_STYLE_RAW => {}
            _ => reasons.push("Неизвестная схема разделов".into()),
        }
    }
    reasons.sort();
    reasons.dedup();
    Ok(Disk {
        path: format!("\\\\.\\PhysicalDrive{number}"),
        model,
        serial,
        size: size as u64,
        bus,
        reasons,
        device_id: number.to_string(),
        sector_size: sector,
    })
}
pub fn list() -> Result<Vec<Disk>> {
    let volumes = volumes()?;
    let mut out = vec![];
    for n in 0..128 {
        let path = format!("\\\\.\\PhysicalDrive{n}");
        match open(&path, false) {
            Ok(f) => out.push(describe(&f, n, &volumes)?),
            Err(e) => {
                if e.downcast_ref::<std::io::Error>()
                    .is_some_and(|e| matches!(e.raw_os_error(), Some(2 | 3 | 21 | 55)))
                {
                    continue;
                }
                return Err(e).with_context(|| format!("Нельзя проверить {path}"));
            }
        }
    }
    Ok(out)
}
pub struct Guard {
    pub file: File,
    volumes: Vec<File>,
}
impl Drop for Guard {
    fn drop(&mut self) {
        for file in &self.volumes {
            let _ = ioctl(file, FSCTL_UNLOCK_VOLUME, &[], 0);
        }
    }
}
pub fn lock(selected: &Disk) -> Result<Guard> {
    let number = selected.device_id.parse::<u32>()?;
    if selected.path != format!("\\\\.\\PhysicalDrive{number}") {
        bail!("Invalid whole-disk path");
    }
    let volumes = volumes()?;
    let file = open(&selected.path, true)?;
    let current = describe(&file, number, &volumes)?;
    crate::usb::same_disk(selected, &current)?;
    if !current.eligible() {
        bail!("{}", current.reasons.join("; "));
    }
    let mut guard = Guard {
        file,
        volumes: vec![],
    };
    for v in volumes.iter().filter(|v| v.disks.contains(&number)) {
        let f = open(v.path.trim_end_matches('\\'), true)?;
        ioctl(&f, FSCTL_LOCK_VOLUME, &[], 0)
            .context("Не удалось заблокировать том; диск не очищен")?;
        guard.volumes.push(f);
        ioctl(
            guard.volumes.last().context("Volume lock")?,
            FSCTL_DISMOUNT_VOLUME,
            &[],
            0,
        )?;
    }
    crate::usb::same_disk(selected, &describe(&guard.file, number, &volumes)?)?;
    Ok(guard)
}
pub fn refresh(file: &File) -> Result<()> {
    ioctl(file, IOCTL_DISK_UPDATE_PROPERTIES, &[], 0)?;
    Ok(())
}
