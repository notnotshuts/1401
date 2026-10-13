# Ferrite: sources and notices

Ferrite is an independent fork of [1401](https://github.com/nullmoth/1401). Project code remains under [PolyForm Noncommercial 1.0.0](LICENSE). Third-party material retains its original notices and license terms. Ferrite does not use or license the NullMoth brand or logo.

## Hardware tables and adapted rules

| Source | Pinned revision | Material retained | License |
|---|---|---|---|
| [OpCore-Simplify](https://github.com/lzhoang2801/OpCore-Simplify) | `e5d8a9f551b1e2a96e2f968696b460b65a7dad2e` | PCI/codec, kext, CPU, SMBIOS and platform tables; adapted compatibility, selection and configuration rules | [BSD-3-Clause](licenses/OpCore-Simplify.txt), Copyright (c) 2024, lzhoang2601 |
| [Hardware-Sniffer](https://github.com/lzhoang2801/Hardware-Sniffer) | `6570f91c33c63f5bf481db31789f903b0d12099f` | GPU identification ranges and report format | [BSD-3-Clause](licenses/Hardware-Sniffer.txt), Copyright (c) 2024, lzhoang2601 |
| NVIDIA open-gpu-kernel-modules | `610.57.04` (inherited table) | Supported PCI IDs and source/test metadata in `nvidia-gsp.json` | Source attribution retained in the dataset; no NVIDIA driver bundled |

Runtime JSON tables live in `crates/ferrite-core/data`. Only the three upstream Python files required to regenerate PCI/codec IDs and GPU ranges are retained under `tools/data_sources`; their provenance and SHA-256 are recorded in `tools/data_sources/PINNED.json`. They are development inputs, not executable runtime dependencies. The exporter preserves the independent NVIDIA table. Other bundled JSON manifests retain their pinned URLs, versions and hashes.

Full upstream applications and the legacy 1401 implementations are no longer present in the current source tree. Their history remains in Git; this does not remove the licenses applying to derived data and adapted code. Source/binary distributions include both BSD license texts.

## Apple Recovery and OpenCore

The Recovery protocol, chunklist format and public verification key derive from OpenCorePkg's `Utilities/macrecovery/macrecovery.py`, Copyright (c) 2019, vit9696, BSD-3-Clause; the original utility credits zhangyoufu for key recovery. The Rust implementation adapts the existing 1401 recovery implementation. This public key verifies signatures; it is not a decryption key. The upstream [BSD notice](licenses/OpenCorePkg.txt) is retained.

OpenCore, its utilities and kexts are downloaded from the pinned manifests and checked with SHA-256. They retain their authors' licenses. Apple Recovery is downloaded directly from Apple's servers and verified against its RSA-signed chunklist and per-block hashes. Neither Recovery images nor downloaded EFI/kext packages are included in this source tree or the Ferrite application packages.

BIOS guidance uses technical facts from the [Dortania OpenCore Install Guide](https://dortania.github.io/OpenCore-Install-Guide/); Ferrite's guide text is independently written.

## Rust and packaging dependencies

Rust dependencies and resolved versions are recorded in `Cargo.lock`; each dependency retains its own license. The desktop uses egui/eframe rather than the former CustomTkinter interface. Python is used only by development export/packaging tools, not by the application runtime.

macOS bundles use local ad-hoc signing. This is not Developer ID signing or Apple notarization. Ferrite is not affiliated with Apple, the OpenCore authors, or the authors of its hardware datasets.

## fatfs

The patched fatfs 0.3.6 source in `vendor/fatfs` is distributed under MIT. See `licenses/fatfs.txt` and `vendor/fatfs/PATCHES.md`.
