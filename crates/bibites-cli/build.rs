//! Build script for the `rustibites` binary.
//!
//! On Windows/MSVC it embeds the red-tinted project icon and version metadata
//! from `assets/icon/rustibites.rc` into the executable. If a resource
//! compiler cannot be found the build still succeeds (with a warning), so the
//! repository remains buildable on machines without the Windows SDK.

use std::path::PathBuf;
use std::process::Command;

fn main() {
    let manifest_dir = PathBuf::from(
        std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is always set by cargo"),
    );
    let workspace_root = manifest_dir.join("..").join("..");
    let icon_dir = workspace_root.join("assets").join("icon");
    let rc_file = icon_dir.join("rustibites.rc");
    let ico_file = icon_dir.join("rustibites.ico");

    println!("cargo:rerun-if-changed={}", rc_file.display());
    println!("cargo:rerun-if-changed={}", icon_dir.display());

    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if target_os != "windows" || target_env != "msvc" {
        return;
    }

    if !rc_file.is_file() || !ico_file.is_file() {
        println!("cargo:warning=assets/icon resources missing; using the default executable icon");
        return;
    }

    let Some(rc) = find_resource_compiler() else {
        println!("cargo:warning=rc.exe not found; skipping icon/version resources");
        return;
    };

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR is set by cargo"));
    let res_file = out_dir.join("rustibites.res");

    let status = Command::new(&rc)
        .current_dir(&icon_dir)
        .arg("/nologo")
        .arg(format!("/fo{}", res_file.display()))
        .arg("rustibites.rc")
        .status();

    match status {
        Ok(status) if status.success() => {
            // MSVC's link.exe accepts .res files directly.
            println!("cargo:rustc-link-arg={}", res_file.display());
        }
        Ok(status) => println!("cargo:warning=rc.exe failed with status {status}"),
        Err(error) => println!("cargo:warning=could not run rc.exe: {error}"),
    }
}

/// Locates the Windows SDK resource compiler (`rc.exe`).
fn find_resource_compiler() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("RC") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Some(path);
        }
    }

    let program_files_x86 = std::env::var_os("ProgramFiles(x86)")?;
    let kits = PathBuf::from(program_files_x86)
        .join("Windows Kits")
        .join("10")
        .join("bin");
    let mut versions: Vec<PathBuf> = std::fs::read_dir(&kits)
        .ok()?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.is_dir())
        .collect();
    versions.sort();
    versions
        .into_iter()
        .rev()
        .map(|version| version.join("x64").join("rc.exe"))
        .find(|candidate| candidate.is_file())
}
