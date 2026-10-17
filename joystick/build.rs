use std::{env, fs, path::PathBuf};

// Put `memory.x` in the linker search path: in a workspace the linker runs
// from the workspace root, so it can't find the file next to Cargo.toml.
fn main() {
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    fs::copy("memory.x", out.join("memory.x")).unwrap();
    println!("cargo:rustc-link-search={}", out.display());
    println!("cargo:rerun-if-changed=memory.x");
}
