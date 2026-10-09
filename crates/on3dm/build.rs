use std::path::PathBuf;

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let src = manifest.join("../../third_party/opennurbs").canonicalize().expect("third_party/opennurbs is missing");
    println!("cargo:rerun-if-changed={}", src.join("CMakeLists.txt").display());
    println!("cargo:rerun-if-changed={}", manifest.join("shim/on3dm_shim.cpp").display());

    let dst = cmake::Config::new(&src)
        .define("CMAKE_POLICY_VERSION_MINIMUM", "3.5")
        .define("CMAKE_POSITION_INDEPENDENT_CODE", "ON")
        .define("CMAKE_BUILD_TYPE", "Release")
        .profile("Release")
        .build_target("opennurbsStatic")
        .build();
    let build = dst.join("build");
    println!("cargo:rustc-link-search=native={}", build.display());
    println!("cargo:rustc-link-search=native={}", build.join("freetype263").display());
    println!("cargo:rustc-link-search=native={}", build.join("zlib").display());
    println!("cargo:rustc-link-search=native={}", build.join("android_uuid").display());

    cc::Build::new()
        .cpp(true)
        .std("c++17")
        .file(manifest.join("shim/on3dm_shim.cpp"))
        .include(&src)
        .define("ON_CMAKE_BUILD", None)
        .warnings(false)
        .flag_if_supported("-Wno-deprecated-declarations")
        .compile("on3dm_shim");

    println!("cargo:rustc-link-lib=static=opennurbsStatic");
    println!("cargo:rustc-link-lib=static=opennurbs_public_freetype");
    println!("cargo:rustc-link-lib=static=zlib");
    println!("cargo:rustc-link-lib=static=android_uuid");
    println!("cargo:rustc-link-lib=dylib=stdc++");
}
