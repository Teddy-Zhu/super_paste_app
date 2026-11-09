use std::path::{Path, PathBuf};

use tauri::{AppHandle, Manager, path::BaseDirectory};

pub fn resolve_ort_runtime(app: &AppHandle) -> Result<PathBuf, String> {
    let (target, library) = runtime_artifact()?;
    let relative_path = Path::new("ort").join(target).join(library);
    let mut candidates = Vec::new();
    let mut resource_error = None;

    if cfg!(debug_assertions) {
        candidates.push(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../resources")
                .join(&relative_path),
        );
    }

    match app.path().resolve(&relative_path, BaseDirectory::Resource) {
        Ok(path) => candidates.push(path),
        Err(error) => resource_error = Some(error.to_string()),
    }

    for candidate in &candidates {
        if !candidate.is_absolute() {
            continue;
        }

        if candidate.is_file() {
            return candidate.canonicalize().map_err(|error| {
                format!(
                    "无法解析 ONNX Runtime 路径 {}: {error}",
                    candidate.display()
                )
            });
        }
    }

    let checked_paths = candidates
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join("; ");
    let resolver_detail = resource_error
        .map(|error| format!("; Tauri resource 路径解析失败: {error}"))
        .unwrap_or_default();

    Err(format!(
        "本地 OCR 的 ONNX Runtime 动态库缺失或路径无效（目标 {target}，期望文件 {library}，已检查: {checked_paths}{resolver_detail}）"
    ))
}

fn runtime_artifact() -> Result<(&'static str, &'static str), String> {
    #[cfg(all(target_os = "windows", target_arch = "x86_64", target_env = "msvc"))]
    {
        return Ok(("x86_64-pc-windows-msvc", "onnxruntime.dll"));
    }

    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        return Ok(("x86_64-apple-darwin", "libonnxruntime.dylib"));
    }

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        return Ok(("aarch64-apple-darwin", "libonnxruntime.dylib"));
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        return Ok(("x86_64-unknown-linux-gnu", "libonnxruntime.so"));
    }

    #[allow(unreachable_code)]
    Err(format!(
        "本地 OCR 暂不支持当前 ONNX Runtime 目标（{}-{}）",
        std::env::consts::OS,
        std::env::consts::ARCH
    ))
}
