use std::path::PathBuf;

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let src = manifest.join("../../third_party/opennurbs").canonicalize().expect("third_party/opennurbs is missing");
    // On Windows canonicalize() returns a `\\?\C:\...` verbatim path, which
    // MSBuild cannot resolve source files against; use the plain form.
    let src = match src.to_str().and_then(|s| s.strip_prefix(r"\\?\")) {
        Some(plain) => PathBuf::from(plain),
        None => src,
    };
    println!("cargo:rerun-if-changed={}", src.join("CMakeLists.txt").display());
    println!("cargo:rerun-if-changed={}", manifest.join("shim/on3dm_shim.cpp").display());

    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap();
    let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    let msvc = target_env == "msvc";

    let mut cfg = cmake::Config::new(&src);
    cfg.define("CMAKE_POLICY_VERSION_MINIMUM", "3.5")
        .define("CMAKE_POSITION_INDEPENDENT_CODE", "ON")
        .define("CMAKE_BUILD_TYPE", "Release")
        .profile("Release")
        .build_target("opennurbsStatic");
    if target_os == "macos" {
        cfg.define("CMAKE_OSX_DEPLOYMENT_TARGET", std::env::var("MACOSX_DEPLOYMENT_TARGET").unwrap_or_else(|_| "11.0".into()));
    }
    let dst = cfg.build();

    // Single-config generators (Make, Ninja) put the libraries straight into
    // each build directory; Visual Studio puts them in a `Release` subfolder.
    let build = dst.join("build");
    for dir in [build.clone(), build.join("freetype263"), build.join("zlib"), build.join("android_uuid")] {
        println!("cargo:rustc-link-search=native={}", dir.display());
        println!("cargo:rustc-link-search=native={}", dir.join("Release").display());
    }

    let mut shim = cc::Build::new();
    shim.cpp(true)
        .std("c++17")
        .file(manifest.join("shim/on3dm_shim.cpp"))
        .include(&src)
        .define("ON_CMAKE_BUILD", None)
        .warnings(false)
        .flag_if_supported("-Wno-deprecated-declarations");
    if msvc {
        shim.define("UNICODE", None).define("_UNICODE", None).flag("/bigobj").flag("/utf-8");
    }
    if target_os == "macos" {
        shim.define("ON_COMPILER_CLANG", None).define("ON_RUNTIME_APPLE", None).define("RHINO_APPLE", "1");
    }
    shim.compile("on3dm_shim");

    println!("cargo:rustc-link-lib=static=opennurbsStatic");
    println!("cargo:rustc-link-lib=static=zlib");
    match target_os.as_str() {
        "linux" | "android" => {
            println!("cargo:rustc-link-lib=static=opennurbs_public_freetype");
            println!("cargo:rustc-link-lib=static=android_uuid");
            println!("cargo:rustc-link-lib=dylib=stdc++");
        }
        "macos" | "ios" => {
            println!("cargo:rustc-link-lib=framework=CoreGraphics");
            println!("cargo:rustc-link-lib=framework=CoreText");
            println!("cargo:rustc-link-lib=framework=Foundation");
            println!("cargo:rustc-link-lib=dylib=c++");
        }
        "windows" => {
            println!("cargo:rustc-link-lib=dylib=shlwapi");
            println!("cargo:rustc-link-lib=dylib=usp10");
            println!("cargo:rustc-link-lib=dylib=ole32");
            println!("cargo:rustc-link-lib=dylib=rpcrt4");
            println!("cargo:rustc-link-lib=dylib=user32");
            println!("cargo:rustc-link-lib=dylib=gdi32");
            println!("cargo:rustc-link-lib=dylib=advapi32");
            println!("cargo:rustc-link-lib=dylib=dwrite");
            println!("cargo:rustc-link-lib=dylib=shell32");
            if !msvc {
                println!("cargo:rustc-link-lib=dylib=stdc++");
            }
        }
        _ => println!("cargo:rustc-link-lib=dylib=stdc++"),
    }
}
