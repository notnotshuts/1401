fn main() {
    println!("cargo:rerun-if-changed=src/macos_disk.c");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        cc::Build::new()
            .file("src/macos_disk.c")
            .flag("-std=c11")
            .warnings(true)
            .compile("ferrite_macos_disk");
        for framework in ["CoreFoundation", "DiskArbitration", "IOKit"] {
            println!("cargo:rustc-link-lib=framework={framework}");
        }
    }
}
