// STA64: lua-src compiles its C sources from mlua-sys's build script, so cargo never sees them
// as inputs of this crate. Watching them here makes an edit to the patched Lua rebuild this
// crate and so rerun mlua-sys's build (see STA64-PATCH.md).
fn main() {
    println!("cargo:rerun-if-changed=lua-5.3.6");
    println!("cargo:rerun-if-changed=build.rs");
}
