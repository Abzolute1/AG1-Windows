fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        let gtk = pkg_config::Config::new()
            .cargo_metadata(false)
            .probe("gtk+-3.0")
            .expect("GTK development headers");
        let drm = pkg_config::Config::new()
            .cargo_metadata(false)
            .probe("libdrm")
            .expect("libdrm development headers");
        cc::Build::new()
            .file("src/rendering_clock.c")
            .includes(gtk.include_paths)
            .includes(drm.include_paths)
            .flag("-pthread")
            .compile("ag1_rendering_clock");
        println!("cargo:rerun-if-changed=src/rendering_clock.c");
        println!("cargo:rustc-link-arg-bin=ag1-desktop=-Wl,--export-dynamic-symbol=drmWaitVBlank");
    }
    tauri_build::build()
}
