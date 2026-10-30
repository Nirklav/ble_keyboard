//! Bundles the keyboard firmware: `KB_FIRMWARE` is the path of an app image (`espflash save-image`
//! output, see `scripts/bundle.sh`), a relative one from the workspace root. The image is checked and copied
//! to `OUT_DIR/firmware.bin`, `cfg(bundled_firmware)` tells the code it's there. Without `KB_FIRMWARE`
//! the app has no firmware of its own, `kb update` takes a file then.

use std::path::{Path, PathBuf};
use std::{env, fs};

use protocol::image;

fn main() {
    println!("cargo::rustc-check-cfg=cfg(bundled_firmware)");
    println!("cargo::rerun-if-env-changed=KB_FIRMWARE");
    let Some(path) = env::var_os("KB_FIRMWARE").filter(|path| !path.is_empty()) else {
        return;
    };
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let path = root.join(PathBuf::from(path));
    println!("cargo::rerun-if-changed={}", path.display());

    let image = fs::read(&path).unwrap_or_else(|error| panic!("KB_FIRMWARE: can't read {}: {error}", path.display()));
    let app = image::app_info(&image)
        .unwrap_or_else(|error| panic!("KB_FIRMWARE: {}: {error} (an ELF file? see scripts/bundle.sh)", path.display()));
    assert!(app.is_for(image::KEYBOARD_PROJECT), "KB_FIRMWARE: {} is not the keyboard firmware: {app:?}", path.display());
    // `firmware::bundled_firmware` needs the build ID too.
    assert!(image::build_id(&image).is_some(), "KB_FIRMWARE: {} has no hash at its end", path.display());

    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("firmware.bin");
    fs::write(&out, &image).unwrap_or_else(|error| panic!("can't write {}: {error}", out.display()));
    println!("cargo::rustc-cfg=bundled_firmware");
    println!("cargo::warning=bundled firmware {} from {}", app.version, path.display());
}
