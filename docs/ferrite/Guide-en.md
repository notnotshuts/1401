# Ferrite — Installation, step by step

Follow the steps in order. BIOS option names may differ on your PC.

## 1. Check your hardware

Scan your PC or import Report.json. Choose macOS and review graphics, network and storage limitations. Building requires an ACPI folder with the DSDT from the same PC.

## 2. Prepare the BIOS

Save your current BIOS settings. If Windows uses BitLocker, save the recovery key before making changes. Do not change RAID/VMD/AHCI without preparing the installed OS.

Only tick settings you have checked in the BIOS. These checkboxes do not change the BIOS.

- [ ] UEFI enabled; CSM/Legacy disabled
- [ ] Secure Boot disabled for OpenCore boot
- [ ] Fast Boot disabled
- [ ] Storage mode checked: AHCI, VMD disabled if needed
- [ ] XHCI Hand-off enabled, if available
- [ ] Above 4G Decoding enabled, if available
- [ ] Re-Size BAR reviewed for the selected GPU
- [ ] Intel: CFG Lock, VT-d and DVMT reviewed against the configuration; iGPU usually needs at least 64 MiB DVMT

## 3. Build the EFI

Open OpenCore EFI, select macOS and your ACPI folder. Choose “Build and check EFI”. Continue after validation succeeds. Complex laptops may need custom ACPI patches.

## 4. Download Apple Recovery

After building, choose “Next: download Apple Recovery”. An internet connection is required. Wait for the download and verification to finish.

## 5. Prepare the USB drive

Connect an 8–128 GiB USB drive and refresh the drive list. Check the model, size and serial number. Back up needed files: writing erases all data on the selected USB. Enter the exact path and start writing. Alternatively, create an image file without modifying drives.

On macOS, select the whole USB disk, such as `/dev/disk4`, rather than a partition such as `/dev/disk4s1`. Confirm the system administrator authorization prompt; Ferrite itself stays unprivileged. If the volume is busy, close Finder windows and terminals using it. Successful writing ejects the drive automatically; reconnect it to inspect EFI. On Apple Silicon, import the target Intel/AMD PC report and ACPI tables.

## 6. Boot and test the result

Open the UEFI Boot Menu and select the USB. Choose Recovery in OpenCore; macOS installation uses the internet. Check the target disk before formatting. After installation, test networking, audio, USB, displays and sleep. Keep the installer USB until internal-disk boot has been tested.

## After installation

- [ ] Ethernet and Wi-Fi
- [ ] Audio and microphone
- [ ] USB ports and connected devices
- [ ] All displays and brightness controls
- [ ] Sleep and wake
- [ ] Boot from the internal drive

## If something goes wrong

Build failed: check the ACPI folder and iasl path. Drive missing: refresh the list and open the blocking reason. Download failed: check your connection and retry. EFI does not boot: review BIOS and hardware; enable the detailed boot log in advanced settings. Error details are available in the activity log.
