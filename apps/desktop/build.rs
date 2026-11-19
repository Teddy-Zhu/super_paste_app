use std::{env, fs, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../../resources/app-icon.ico");
    println!("cargo:rerun-if-changed=../../packaging/windows/app.rc.in");
    println!("cargo:rerun-if-env-changed=RC");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") { return; }

    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("Cargo manifest directory"));
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo build output directory"));
    let icon = root.join("../../resources/app-icon.ico").canonicalize().expect("application icon");
    let icon = icon.to_str().expect("Windows resource paths must be valid Unicode");
    let icon = if let Some(unc) = icon.strip_prefix(r"\\?\UNC\") { format!("//{}", unc.replace('\\', "/")) }
        else { icon.strip_prefix(r"\\?\").unwrap_or(icon).replace('\\', "/") };
    let version = env::var("CARGO_PKG_VERSION").expect("Cargo package version");
    let components = ["CARGO_PKG_VERSION_MAJOR", "CARGO_PKG_VERSION_MINOR", "CARGO_PKG_VERSION_PATCH"]
        .map(|key| env::var(key).expect("Cargo version component").parse::<u16>().expect("Windows version components must fit in u16"));
    let source = fs::read_to_string(root.join("../../packaging/windows/app.rc.in")).expect("Windows resource template")
        .replace("@ICON@", &icon)
        .replace("@VERSION_NUMBER@", &format!("{},{},{},0", components[0], components[1], components[2]))
        .replace("@VERSION_STRING@", &version);
    let resource_source = out.join("super-paste.rc");
    let gnu = env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("gnu");
    let resource = out.join(if gnu { "super-paste-resource.o" } else { "super-paste.res" });
    fs::write(&resource_source, source).expect("write Windows resource source");
    let compiler = env::var_os("RC").unwrap_or_else(|| {
        if !gnu { return "rc.exe".into(); }
        if cfg!(windows) { return "windres".into(); }
        let target = env::var("TARGET").expect("Cargo target triple");
        let architecture = target.split('-').next().expect("target architecture");
        format!("{architecture}-w64-mingw32-windres").into()
    });
    let mut command = Command::new(compiler);
    if gnu {
        // GNU links a COFF object; MSVC consumes the SDK's .res format.
        command.args(["--input-format=rc", "--output-format=coff", "--codepage=65001"])
            .arg("--input").arg(&resource_source).arg("--output").arg(&resource);
    } else {
        command.arg("/nologo").arg("/c65001").arg("/fo").arg(&resource).arg(&resource_source);
    }
    let status = command.status()
        .expect("Windows resource compiler is required (MSVC: rc.exe; GNU: windres); set RC to its path");
    assert!(status.success(), "Windows resource compiler failed: {status}");
    println!("cargo:rustc-link-arg-bin=super-paste={}", resource.display());
}
