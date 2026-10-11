# Быстрый старт: 1401 Linux

1. Скачайте `.deb` из [последнего релиза](https://github.com/notnotshuts/1401/releases/latest).
2. Установите: `sudo apt install ./p1401-linux_*_amd64.deb` и запустите `1401 gui`.
3. На странице «Обзор» нажмите «Подготовить всё автоматически». Подтвердите системный запрос прав для сканирования. Дождитесь проверки железа, сборки EFI и личного гайда.
4. Прочитайте гайд, нажмите «Далее: выбрать USB →», выберите флешку по модели и размеру и подтвердите очистку её точным путём. Все данные на флешке будут удалены.
5. После успешной записи сфотографируйте `BIOS.txt`, настройте BIOS по нему и загрузитесь с USB в режиме UEFI. Дальнейшие шаги установки описаны в гайде.

Для импортированного отчёта выберите папку ACPI именно того ПК. Если проверка остановилась, исправьте указанные проблемы; мастер не обходит ошибки совместимости. Если в списке нет доступной флешки, переподключите её и нажмите «Обновить». Причина запрета записи показана под списком.

Личный гайд и результаты сохраняются в рабочей папке. После изменения железа или BIOS выполните новое сканирование. Приложение не меняет BIOS и не приостанавливает BitLocker. Не переключайте RAID/VMD или Legacy вслепую на ПК с существующей ОС.

---

# 1401 on Linux (x86_64)

This fork adds a CustomTkinter desktop interface, CLI, and Linux USB backend. It uses
the same pinned Hardware Sniffer, OpCore-Simplify engine, policy rules, Apple
recovery verification and OpenCore validator as the Windows utility. The Linux
package uses the system Python interpreter; it is not a port of the WinForms UI.

## Install

Download files from the fork's [latest Linux release](https://github.com/notnotshuts/1401/releases/latest).
Assets include the .deb, portable archive, screenshots, instructions, licences
and SHA256SUMS. Download all assets to verify the complete manifest with
`sha256sum -c SHA256SUMS`. Successful pushes to main (or manual workflow runs
on main) publish releases automatically after all checks pass.
On Ubuntu 22.04+ or Debian 12+:

```sh
sudo apt install ./p1401-linux_*_amd64.deb
1401 gui
```

For other x86_64 distributions, install Python 3.10+, Tkinter, certifi, pciutils,
usbutils, util-linux (including lsblk, mount, umount and sfdisk), dosfstools, udev
sudo, PolicyKit and acpica-tools (`iasl`). CustomTkinter and its dependencies are bundled. Extract the `.tar.gz`, then run `./1401-linux-x86_64/1401`.
The launcher copies its engine into a versioned private cache, so packages under
`/usr/lib` stay writable only to the package manager and downloads work as a
normal user. Native builds and package smoke tests run on Ubuntu 22.04 x86_64.

## Hardware check and desktop interface

The CustomTkinter desktop provides overview, hardware compatibility, EFI build,
BIOS guide, USB selection and operation log pages. Import a Hardware Sniffer
report or scan the local PC; choose macOS by name and inspect per-device support.
The read-only `check` command uses the pinned OpCore-Simplify compatibility checker
with 1401's NVIDIA policy, without compiling ACPI or downloading components.
Known device ranges, OCLP requirements, blockers and unchecked devices are shown
separately. The engine's actual EFI build remains authoritative.

The installed desktop invokes `/usr/bin/1401` through `pkexec` for scan and USB
write. PolicyKit asks for administrative authentication. If unavailable, the app
copies a shell-quoted sudo command for a terminal. A compatibility check alone
needs no administrative access. Reports, target version and DSDT are checked
before enabling build; changed inputs invalidate the previous assessment.

## Command line

Commands use absolute paths so privileged scanning and ordinary builds share the
same inputs. Replace `/path/to/work` and `/dev/sdX` with your own paths.

```sh
sudo 1401 scan /path/to/work/scan
1401 check /path/to/work/scan/Report.json --macos 24 --output /path/to/work/compatibility.json
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
internet to fetch OpenCore, kexts and ACPI data. The ACPI compiler `iasl` comes from the distribution’s acpica-tools package. No automatic log submission is
added by this port.

## USB safeguards and limits

`/dev/sda` — весь накопитель; `/dev/sda1` — раздел на нём. Для создания установочного USB выберите и подтвердите весь накопитель: разметка удаляет все его разделы. Монтируется только новый раздел FAT32. Его путь программа получает из дерева устройств Linux, проверяет номер, размер и принадлежность накопителю; после монтирования сверяет источник через `findmnt`. Интерфейс показывает существующие разделы, а журнал — фактически смонтированный раздел.

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
collector. The CI installer test downloads a real Apple-signed Recovery,
writes an owned loop image using the installed backend, remounts it read-only
and rechecks EFI files and all signed Recovery chunks. OVMF/QEMU then boots
the USB-attached image to the OpenCore Recovery picker. The evidence archive
contains screenshots, logs and hashes, never Apple files or disk contents.
Only the CI copy has a persistent picker and serial debugging enabled.
This does not verify physical USB hardware or booting the macOS kernel. Real hardware validation
is still required before claiming end-to-end macOS support.

## Build from source

```sh
python3 -m unittest discover -s tests -v
python3 -m pip install --require-hashes --no-deps --target build/linux-vendor -r linux/requirements.txt
python3 tools/linux_release.py --output dist/linux --vendor build/linux-vendor
```

The workflow builds a `.deb`, a portable Python application archive and
`SHA256SUMS`. Upstream source and third-party notices are included. Original
PolyForm Noncommercial and third-party licences apply; no logo or trademark
rights are granted by this fork.


## Linux Mint: обновление после ошибки сканирования

Установите свежий `.deb` поверх предыдущего: `sudo apt install ./p1401-linux_*_amd64.deb`. Закройте старое окно и запустите `1401 gui` обычным пользователем. Нажмите «Сканировать этот ПК»: новый запуск создаст отдельную папку и вернёт владельца файлов вашему пользователю. Затем используйте «Подготовить всё автоматически».

Если Report.json импортирован, его папка ACPI должна быть от того же ПК. Отчёт без DSDT можно проверить, но из него нельзя собрать EFI. Откройте «Журнал» для конкретной причины, если сканирование осталось частичным.
