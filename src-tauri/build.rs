use std::{env, fs, path::PathBuf};

fn main() {
    validate_ort_bundle_resource();
    tauri_build::build()
}

fn validate_ort_bundle_resource() {
    let target = env::var("TARGET").expect("Cargo did not provide TARGET");
    let library = match target.as_str() {
        "x86_64-pc-windows-msvc" => "onnxruntime.dll",
        "x86_64-apple-darwin" | "aarch64-apple-darwin" => "libonnxruntime.dylib",
        "x86_64-unknown-linux-gnu" => "libonnxruntime.so",
        _ => panic!("unsupported ONNX Runtime release target: {target}"),
    };
    let project_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let ort_dir = project_dir.join("../resources/ort").join(&target);
    let manifest = ort_dir.join("runtime-manifest.json");
    let runtime = ort_dir.join(library);
    let license = ort_dir.join("LICENSE");
    let notices = ort_dir.join("ThirdPartyNotices.txt");

    for path in [&manifest, &runtime, &license, &notices] {
        println!("cargo:rerun-if-changed={}", path.display());
        if !path.is_file() {
            panic!(
                "missing ONNX Runtime release resource {}. Run `pnpm prepare:ort` from super_paste before building.",
                path.display()
            );
        }
    }

    let manifest_text = fs::read_to_string(&manifest).unwrap_or_else(|error| {
        panic!(
            "failed to read ONNX Runtime manifest {}: {error}",
            manifest.display()
        )
    });
    for expected in [
        format!("\"target\": \"{target}\""),
        "\"runtimeVersion\": \"1.28.0\"".to_owned(),
        "\"ortApiVersion\": 17".to_owned(),
    ] {
        if !manifest_text.contains(&expected) {
            panic!(
                "ONNX Runtime manifest {} does not match the pinned runtime contract ({expected})",
                manifest.display()
            );
        }
    }

    println!("cargo:rerun-if-env-changed=TARGET");
}
