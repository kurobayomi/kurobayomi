//! Compiles the bundled icon theme (`src/theme/icons`) into a GResource under
//! `/io/github/anas1412/Kurobayomi/icons`; `theme::icons::register` loads it at startup.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

const PREFIX: &str = "/io/github/anas1412/Kurobayomi/icons";

fn files(dir: &Path, root: &Path, out: &mut Vec<String>) {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir).expect("read icon dir").map(|e| e.expect("dir entry").path()).collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            files(&path, root, out);
        } else if path.extension().is_some_and(|e| e == "svg") {
            // resource names use `/` on every system (Windows would give `actions\x.svg`, which no lookup finds)
            let relative = path.strip_prefix(root).expect("under root").components();
            out.push(relative.map(|part| part.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/"));
        }
    }
}

/// Copies the icon tree to `to` with the logos' frames in gold: the in-app logo of a nightly build (the welcome page,
/// the app icon). The window and desktop icons of the AppImage are `src/shell/data/icons-nightly`.
fn nightly_icons(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("create nightly icon dir");
    for entry in std::fs::read_dir(from).expect("read icon dir") {
        let path = entry.expect("dir entry").path();
        let target = to.join(path.file_name().expect("file name"));
        if path.is_dir() {
            nightly_icons(&path, &target);
        } else if path.file_name().is_some_and(|n| n == "kb-logo-shadow.svg" || n == "io.github.anas1412.Kurobayomi.svg") {
            // the app icon also takes the nightly app id as its name (`channel::APP_ID`)
            let target = if path.file_name().is_some_and(|n| n == "io.github.anas1412.Kurobayomi.svg") { to.join("io.github.anas1412.Kurobayomi.Nightly.svg") } else { target };
            let svg = std::fs::read_to_string(&path).expect("read logo");
            std::fs::write(&target, svg.replace("stroke=\"#ffffff\" stroke-width=", "stroke=\"#f2b01e\" stroke-width=")).expect("write logo");
        } else if path.file_name().is_some_and(|n| n == "io.github.anas1412.Kurobayomi-symbolic.svg") {
            std::fs::copy(&path, to.join("io.github.anas1412.Kurobayomi.Nightly-symbolic.svg")).expect("copy icon");
        } else {
            std::fs::copy(&path, &target).expect("copy icon");
        }
    }
}

/// The app icon as a Windows `.ico`: the channel's hicolor PNGs (16 to 256 px) stored as they are, which Windows reads
/// since Vista. Six directory entries, then the images.
fn windows_icon(nightly: bool, to: &Path) {
    let (dir, name) = if nightly {
        ("src/shell/data/icons-nightly", "io.github.anas1412.Kurobayomi.Nightly")
    } else {
        ("src/shell/data/icons", "io.github.anas1412.Kurobayomi")
    };
    let sizes = [16u32, 24, 32, 48, 128, 256];
    let images: Vec<Vec<u8>> = sizes
        .iter()
        .map(|size| {
            let png = format!("{dir}/hicolor/{size}x{size}/apps/{name}.png");
            println!("cargo:rerun-if-changed={png}");
            std::fs::read(&png).expect("read app icon")
        })
        .collect();
    let mut ico = Vec::new();
    for field in [0u16, 1, sizes.len() as u16] {
        ico.extend(field.to_le_bytes());
    }
    let mut offset = 6 + 16 * sizes.len() as u32;
    for (size, png) in sizes.iter().zip(&images) {
        // a side of 256 is written as 0
        let side = u8::try_from(*size).unwrap_or(0);
        ico.extend([side, side, 0, 0]);
        ico.extend(1u16.to_le_bytes()); // colour planes
        ico.extend(32u16.to_le_bytes()); // bits per pixel
        ico.extend((png.len() as u32).to_le_bytes());
        ico.extend(offset.to_le_bytes());
        offset += png.len() as u32;
    }
    for png in &images {
        ico.extend(png);
    }
    std::fs::write(to, ico).expect("write app icon");
}

fn main() {
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let source = PathBuf::from("src/theme/icons");
    println!("cargo:rerun-if-changed=src/theme/icons");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=KB_CHANNEL");
    let nightly = std::env::var("KB_CHANNEL").is_ok_and(|channel| channel == "nightly");
    // Windows takes a program's icon (taskbar, Explorer, title bar) from the exe; without one GTK shows its own logo
    if std::env::var("CARGO_CFG_TARGET_OS").is_ok_and(|os| os == "windows") {
        let ico = out.join("kurobayomi.ico");
        windows_icon(nightly, &ico);
        let script = out.join("kurobayomi.rc");
        std::fs::write(&script, format!("1 ICON \"{}\"\n", ico.display().to_string().replace('\\', "/"))).expect("write resource script");
        let object = out.join("kurobayomi-resources.o");
        // ponytail: windres of the MinGW toolchain (the MSYS2 build); an MSVC build would need rc.exe
        let status = Command::new("windres")
            .arg(&script)
            .args(["-O", "coff", "-o"])
            .arg(&object)
            .status()
            .expect("windres must be installed (MinGW binutils)");
        assert!(status.success(), "windres failed");
        println!("cargo:rustc-link-arg-bins={}", object.display());
    }
    let root = if nightly {
        nightly_icons(&source, &out.join("icons"));
        out.join("icons")
    } else {
        source
    };
    let mut list = Vec::new();
    files(&root, &root, &mut list);
    let mut xml = format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<gresources>\n  <gresource prefix=\"{PREFIX}\">\n");
    for f in &list {
        let _ = writeln!(xml, "    <file>{f}</file>");
        println!("cargo:rerun-if-changed=src/theme/icons/{f}");
    }
    xml.push_str("  </gresource>\n</gresources>\n");
    let xml_path = out.join("kurobayomi.gresource.xml");
    std::fs::write(&xml_path, xml).expect("write gresource xml");
    let status = Command::new("glib-compile-resources")
        .arg(format!("--sourcedir={}", root.display()))
        .arg(format!("--target={}", out.join("kurobayomi.gresource").display()))
        .arg(&xml_path)
        .status()
        .expect("glib-compile-resources must be installed (part of the GLib development tools)");
    assert!(status.success(), "glib-compile-resources failed");
}
