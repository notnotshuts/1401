# 1401 on Linux (x86_64)

This fork adds a Python/Tk desktop interface, CLI, and Linux USB backend. It uses
the same pinned Hardware Sniffer, OpCore-Simplify engine, policy rules, Apple
recovery verification and OpenCore validator as the Windows utility. The Linux
package uses the system Python interpreter; it is not a port of the WinForms UI.

## Install

Download the `1401-linux-x86_64` artifact from the fork's successful **Linux build
and verification** Actions run. Unzip it and verify `sha256sum -c SHA256SUMS`.
On Ubuntu 22.04+ or Debian 12+:

```sh
sudo apt install ./p1401-linux_*_amd64.deb
1401 gui
```

For other x86_64 distributions, install Python 3.10+, Tkinter, certifi, pciutils,
usbutils, util-linux (including lsblk, mount, umount and sfdisk), dosfstools, udev
and sudo. Extract the `.tar.gz`, then run `./1401-linux-x86_64/1401`.
The launcher copies its engine into a versioned private cache, so packages under
`/usr/lib` stay writable only to the package manager and downloads work as a
normal user. Native builds and package smoke tests run on Ubuntu 22.04 x86_64.

## Command line

Commands use absolute paths so privileged scanning and ordinary builds share the
same inputs. Replace `/path/to/work` and `/dev/sdX` with your own paths.

```sh
sudo 1401 scan /path/to/work/scan
1401 plan /path/to/work/scan/Report.json /path/to/work/scan/ACPI --macos 24
1401 build /path/to/work/scan/Report.json /path/to/work/scan/ACPI /path/to/work/build --macos 24
1401 guide /path/to/work/scan/Report.json /path/to/work/build /path/to/work/BIOS.txt
1401 disks
sudo 1401 write /dev/sdX /path/to/work/build \
  --profile /path/to/work/scan/Report.json --darwin 24 --confirm-erase /dev/sdX
```

`--macos` / `--darwin` use Darwin major versions: 23 = Sonoma 14, 24 = Sequoia 15,
25 = Tahoe 26. Review the build result and BIOS guide before booting the USB.
Scanning requires access to `/sys/firmware/acpi/tables`; containers, VMs without
ACPI, ARM and non-x86 machines are not supported hardware targets. Builds need
internet to fetch OpenCore, kexts and ACPI tools. No automatic log submission is
added by this port.

## USB safeguards and limits

**Writing erases the selected USB disk.** The command requires root and a second
copy of its exact device path. It refuses internal, read-only, system-mounted,
OS-containing, holder-backed and unidentified disks, and rechecks serial, model,
size and kernel device identity before partitioning. Active media mounts must
unmount successfully. Disks above 256 GiB require `--allow-large`. Recovery and
any NVIDIA companion payload are downloaded before erasure; the EFI and build
profile must pass the existing verification gates first.

The layout is MBR with one bootable FAT32 partition capped at 16 GiB, containing
`EFI` and `com.apple.recovery.boot`. Files are flushed and read back before the
volume is unmounted. An interrupted write leaves an incomplete installer: run
the whole write command again. Disks with no serial or existing EFI system / OS
partitions are deliberately refused; use a different empty USB stick.

Windows-only native diagnostics (NVML, firmware resource inventory and the
Windows CPU helper) remain unavailable on Linux; they must not be interpreted
as measured Linux facts. The Linux hardware scan uses the upstream sysfs/PCI
collector. The CI loop-image test verifies storage mechanics, not physical USB
hardware, firmware boot or installation on a real PC. Real hardware validation
is still required before claiming end-to-end macOS support.

## Build from source

```sh
python3 -m unittest discover -s tests -v
python3 tools/linux_release.py --output dist/linux
```

The workflow builds a `.deb`, a portable Python application archive and
`SHA256SUMS`. Upstream source and third-party notices are included. Original
PolyForm Noncommercial and third-party licences apply; no logo or trademark
rights are granted by this fork.
