// Only enabled backends are compiled and linked. See docs/licences.md.
// X264_SYSROOT points at a directory holding usr/include/x264.h, for cross builds.
fn main() {
    if std::env::var_os("CARGO_FEATURE_X264").is_some() {
        build_x264();
    }
    if std::env::var_os("CARGO_FEATURE_OPENH264").is_some() {
        build_openh264();
    }
}

fn build_x264() {
    println!("cargo:rerun-if-changed=csrc/x264_shim.c");
    println!("cargo:rerun-if-env-changed=X264_SYSROOT");
    let mut build = cc::Build::new();
    build.file("csrc/x264_shim.c").opt_level(2).warnings(true);
    if let Ok(root) = std::env::var("X264_SYSROOT") {
        build.include(format!("{root}/usr/include"));
        let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
        let multiarch = if arch == "aarch64" {
            "aarch64-linux-gnu"
        } else {
            "x86_64-linux-gnu"
        };
        build.include(format!("{root}/usr/include/{multiarch}"));
        println!("cargo:rustc-link-search=native={root}/usr/lib/{multiarch}");
    }
    build.compile("x264_shim");
    println!("cargo:rustc-link-lib=dylib=x264");
}

fn build_openh264() {
    println!("cargo:rerun-if-changed=csrc/openh264_shim.cpp");
    println!("cargo:rerun-if-env-changed=OPENH264_ROOT");
    let root = std::env::var("OPENH264_ROOT").expect(
        "openh264 needs OPENH264_ROOT: build the pinned source with csrc/build-openh264.sh",
    );
    let provenance = format!("{root}/share/doc/openh264/SOURCE");
    println!("cargo:rerun-if-changed={provenance}");
    assert_eq!(
        std::fs::read_to_string(&provenance)
            .expect("missing OpenH264 source provenance")
            .trim(),
        "v2.7.0 1bca62311e447fd9c39fc93de2c4b96cd5ce971c",
        "OpenH264 must be built from the pinned source",
    );
    println!("cargo:rerun-if-changed={root}/lib/libopenh264.a");
    cc::Build::new()
        .cpp(true)
        .std("c++11")
        .file("csrc/openh264_shim.cpp")
        .include(format!("{root}/include"))
        .opt_level(2)
        .warnings(true)
        .cpp_link_stdlib(None)
        .compile("openh264_shim");
    println!("cargo:rustc-link-search=native={root}/lib");
    println!("cargo:rustc-link-lib=static=openh264");
    // The C++ runtime is supplied by the base system, like glibc.
    println!("cargo:rustc-link-lib=dylib=stdc++");
}
