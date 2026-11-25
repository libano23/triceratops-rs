fn main() {
    // Embed the application icon and version info in the Windows executable.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rerun-if-changed=../../packaging/windows/triceratops.ico");
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../../packaging/windows/triceratops.ico");
        if let Err(e) = res.compile() {
            println!("cargo:warning=could not embed the Windows icon: {e}");
        }
    }
}
