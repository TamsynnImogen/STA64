//! Embeds the STA64 icon and version info into the Windows executable.

fn main() {
    println!("cargo:rerun-if-changed=../../assets/icons/STA64.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../../assets/icons/STA64.ico");
        res.set("ProductName", "STA64");
        res.set("FileDescription", "STA64");
        if let Err(e) = res.compile() {
            println!("cargo:warning=could not embed Windows icon: {e}");
        }
    }
}
