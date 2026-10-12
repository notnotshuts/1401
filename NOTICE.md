# NOTICE - what 1401 is built on

1401 is free, forever: **PolyForm Noncommercial 1.0.0** (`LICENSE`), never for sale. It stands on work other people
gave away, and every piece keeps its own license and credit below.

## Loader dependencies (Rust crates, compiled into `loader1401.efi`, from `loader/Cargo.lock`)

| Crate | License |
|---|---|
| `uefi`, `uefi-raw`, `uefi-macros`, `uguid`, `log`, `bitflags`, `cfg-if`, `bit_field` | MIT or Apache-2.0 |
| `ptr_meta` | MIT |
| `ucs2` | MPL-2.0 |
| build-time only: `syn`, `quote`, `proc-macro2`, `unicode-ident` | MIT or Apache-2.0 (+ Unicode-3.0) |

Test-only, never shipped: QEMU and its EDK2/OVMF firmware (`tests/loader_qemu.py`).

## Vendored source (in `upstream/`, pinned in `upstream/PINNED.json`, unmodified)

| Component | Author | License | Role in 1401 |
|---|---|---|---|
| OpCore Simplify | lzhoang2801 (LICENSE: "Copyright (c) 2024, lzhoang2601") | BSD-3-Clause | The EFI engine: compatibility, SSDTs, kext selection, config.plist. 1401 drives it headlessly and post-processes its output. |
| Hardware Sniffer | lzhoang2801 (same) | BSD-3-Clause | The hardware scan: Report.json plus the ACPI dump. |

Both license texts sit beside the code in `upstream/*/LICENSE`. 1401 never edits upstream files. Every change 1401
makes to their output lives in `p1401/` and is recorded per build as a named rule.

## Code adapted into 1401

| Where | From | License |
|---|---|---|
| `p1401/apple.py`: the recovery protocol, the chunklist format, Apple's EFI ROM public key | OpenCorePkg `Utilities/macrecovery/macrecovery.py`, Copyright (c) 2019, vit9696 (key recovery credited there to zhangyoufu) | BSD-3-Clause |

## Facts cross-checked, no text copied

| Where | Checked against | License of the source |
|---|---|---|
| `p1401/guide.py`: which BIOS settings macOS needs and why | Dortania, *OpenCore Install Guide* | CC BY-NC-SA 4.0. 1401 uses its facts only; every sentence in the guide is 1401's own, because NC-SA text cannot go into a GPL-3 program. |

## Downloaded at build or run time, verified, never redistributed by 1401

| What | From | How it is verified |
|---|---|---|
| OpenCorePkg release (bootloader, drivers, `ocvalidate`) | acidanthera, via the engine's pinned build repo | sha256 recorded in the engine's download history. `ocvalidate` refuses to run without one. |
| Kexts (Lilu, WhateverGreen, AppleALC, itlwm, ...) | Each author's release, chosen by the engine | Each keeps its own license. **Two currently have no sha256 in the engine's manifest (AppleIGB, AppleMCEReporterDisabler), which is an open 1401 issue.** |
| `acpidump.exe` (ACPICA R2024_12_12) | Intel ACPICA GitHub release | sha256 pinned in `p1401/scan.py`. ACPICA is dual BSD-3 / GPL-2. |
| macOS recovery image | **Apple**, from `osrecovery.apple.com` to the user's own USB stick | Apple's RSA-signed chunklist, with every chunk hash-checked as it streams |

## The NullMoth brand is not licensed

The license covers 1401's software and nothing else. The names **NullMoth** and **NullMoth Systems** and the moth logo
(`p1401/moth-mark.jpg`) are © 2026 NullMoth Systems, all rights reserved; no trademark rights are granted.

- Unmodified copies of 1401 may carry them exactly as shipped.
- A modified version must drop them: replace or delete `p1401/moth-mark.jpg` (or render with `mark=b""`), take the
  NullMoth names off the pages it generates, and never present itself as NullMoth's.

## Apple

1401 contains no Apple software and serves none. macOS downloads from Apple's servers directly onto the user's own
machine. 1401 ships no decryption keys and circumvents no Apple access control. The macOS license terms are
between the user and Apple, and 1401's README says so plainly.

## Linux desktop dependencies

The Linux package bundles the following pinned Python wheels, including their
`.dist-info` licence files and CustomTkinter's required themes/fonts:

| Component | Version | Licence | Source |
|---|---|---|---|
| CustomTkinter | 5.2.2 | MIT | https://github.com/TomSchimansky/CustomTkinter |
| darkdetect | 0.8.0 | BSD-3-Clause | https://github.com/albertosottile/darkdetect |
| packaging | 24.2 | Apache-2.0 OR BSD-2-Clause | https://github.com/pypa/packaging |

Wheel hashes are fixed in `linux/requirements.txt`. No runtime pip installation
is required by the packaged desktop. The Linux application uses its own text
branding and does not display the upstream moth logo.


## Linux scanner adaptations

`p1401/linux_scan.py` wraps the unchanged pinned Hardware Sniffer: physical CPU topology, PCI graphics fallback, resilient section collection, readable scan diagnostics, DRM connector parsing and binary EFI SecureBoot variables. `p1401/report.py` extends the validator instance to preserve an unknown Secure Boot state and accepts omitted empty Network / USB groups. It never substitutes missing CPU, GPU or disk observations with supported hardware.

## Ferrite Rust migration

`crates/ferrite-core/data/pci-rules.json` derives from the pinned OpCore-Simplify PCI and audio codec datasets. `gpu-ranges.json` derives from Hardware-Sniffer's GPUIdentifier, preserving ordered identification results over all 16-bit device IDs. Both upstreams retain their BSD-3-Clause notices in their respective `upstream/*/LICENSE` files. Rust compatibility logic is adapted from the pinned compatibility checker; the original copyright/license notices remain applicable. `nvidia-gsp.json` preserves the existing 1401 NVIDIA table and its source/tested metadata. `tools/ferrite/export_rules.py` reproduces the exports and is not part of the runtime.

Ferrite is an independent name for this fork. The new code remains under the repository's PolyForm Noncommercial 1.0.0 license. It does not claim ownership of third-party datasets or NullMoth branding.

Ferrite 0.2 additionally exports the pinned kext catalog, codec layouts, CPU identifiers and model/platform tables into Rust JSON data. Selection/configuration rules adapt the pinned OpCore-Simplify/1401 implementations. The Rust Apple recovery implementation preserves the Apple firmware verification public key and protocol from the existing recovery code; the public key is verification data, not a decryption key. Downloaded OpenCore/kext/Apple artifacts retain their own licenses and are not bundled in this source archive. Rust dependencies are pinned in Cargo.lock and retain their own license terms.
