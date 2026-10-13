# fatfs 0.3.6 compatibility patch

Source: crates.io fatfs 0.3.6 (rafalh/rust-fatfs); original MIT license preserved.
Only source modules, the build manifest and license are vendored.

- Write `.` and `..` without long-name entries so they occupy the first two directory slots.
- Encode `..` pointing to the FAT32 root as cluster zero, as required by the FAT specification.

Apple fsck_msdos independently rejected the original directory layout. Ferrite tests the patched output with native filesystem checking and mounting in Actions.
