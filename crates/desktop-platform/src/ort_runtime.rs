//! Resolves this application's prepared resources; never searches cwd, PATH or the old app.
use std::{collections::BTreeMap, path::{Component, Path, PathBuf}};
use serde::Deserialize;

const MANAGED_BY: &str = "super-paste-rs/scripts/prepare-ort.mjs";
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeSpec { ort_api_version: u32, release_assets: BTreeMap<String, Release> }
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Release { runtime_version: String, library: String }
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Prepared { managed_by: String, runtime_version: String, ort_api_version: u32, target: String, libraries: Vec<File>, notices: Vec<File> }
#[derive(Deserialize)]
struct File { file: String, sha256: String }

pub fn resolve_ort_runtime() -> Result<PathBuf, String> {
    let spec: RuntimeSpec = serde_json::from_str(include_str!("../../../resources/ort-runtime.json"))
        .map_err(|error| format!("ONNX Runtime 资源定义无效: {error}"))?;
    let target = target()?;
    let release = spec.release_assets.get(target).ok_or_else(|| format!("缺少 ONNX Runtime 目标定义: {target}"))?;
    if !single_file(&release.library) { return Err("ONNX Runtime 主库名称无效".into()); }
    let roots = resource_roots()?;
    for root in &roots {
        let directory = root.join("ort").join(target);
        let manifest = directory.join("runtime-manifest.json");
        if !directory.exists() { continue; }
        let metadata = std::fs::metadata(&manifest).map_err(|error| format!("无法读取 ONNX Runtime 清单 {}: {error}", manifest.display()))?;
        if metadata.len() > 256 * 1024 { return Err(format!("ONNX Runtime 清单过大: {}", manifest.display())); }
        let prepared: Prepared = serde_json::from_slice(&std::fs::read(&manifest).map_err(|error| error.to_string())?)
            .map_err(|error| format!("ONNX Runtime 清单无效 {}: {error}", manifest.display()))?;
        if prepared.managed_by != MANAGED_BY || prepared.runtime_version != release.runtime_version || prepared.ort_api_version != spec.ort_api_version || prepared.target != target {
            return Err(format!("ONNX Runtime 清单与本应用版本/目标不匹配: {}", manifest.display()));
        }
        if !prepared.libraries.iter().any(|entry| entry.file == release.library)
            || !["LICENSE", "ThirdPartyNotices.txt"].iter().all(|name| prepared.notices.iter().any(|entry| entry.file == *name)) {
            return Err(format!("ONNX Runtime 清单缺少主库或许可证: {}", manifest.display()));
        }
        for entry in prepared.libraries.iter().chain(&prepared.notices) {
            if !single_file(&entry.file) || entry.sha256.len() != 64 || !entry.sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return Err(format!("ONNX Runtime 清单包含无效文件项: {}", manifest.display()));
            }
            if !directory.join(&entry.file).is_file() { return Err(format!("ONNX Runtime 资源文件缺失: {}", directory.join(&entry.file).display())); }
        }
        // Archive/file hashes are checked by preparation and packaging. Startup validates the
        // manifest and required files, without rereading every shared library on each launch.
        return directory.join(&release.library).canonicalize().map_err(|error| format!("无法解析 ONNX Runtime 主库路径: {error}"));
    }
    Err(format!("本地 OCR 的 ONNX Runtime 资源缺失（目标 {target}，已检查: {}）", roots.iter().map(|root| root.join("ort").join(target).display().to_string()).collect::<Vec<_>>().join("; ")))
}

fn single_file(file: &str) -> bool {
    let mut parts = Path::new(file).components();
    !file.is_empty() && matches!(parts.next(), Some(Component::Normal(_))) && parts.next().is_none()
}
fn resource_roots() -> Result<Vec<PathBuf>, String> {
    let executable = std::env::current_exe().map_err(|error| format!("无法定位当前程序: {error}"))?;
    let executable = executable.parent().ok_or("当前程序路径没有父目录")?;
    let mut roots = Vec::new();
    #[cfg(debug_assertions)]
    roots.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../resources"));
    roots.push(executable.join("resources"));
    #[cfg(target_os = "macos")]
    roots.push(executable.join("../Resources"));
    #[cfg(target_os = "linux")]
    roots.push(executable.join("../lib/super-paste"));
    Ok(roots)
}
fn target() -> Result<&'static str, String> {
    #[cfg(all(target_os = "windows", target_arch = "x86_64", target_env = "msvc"))]
    return Ok("x86_64-pc-windows-msvc");
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    return Ok("x86_64-apple-darwin");
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    return Ok("aarch64-apple-darwin");
    #[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]
    return Ok("x86_64-unknown-linux-gnu");
    #[allow(unreachable_code)]
    Err(format!("本地 OCR 暂不支持当前运行库目标（{}-{}）", std::env::consts::OS, std::env::consts::ARCH))
}
