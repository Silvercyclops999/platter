fn main() {
    // The Discord application id is read at compile time.
    println!("cargo:rerun-if-env-changed=PLATTER_DISCORD_APP_ID");
    // The macOS keychain owner is the team the release is signed with.
    println!("cargo:rerun-if-env-changed=APPLE_TEAM_ID");
    link_clang_runtime();
    tauri_build::build()
}

/// whisper.cpp's Metal code checks the macOS version with `@available`,
/// which calls `__isPlatformVersionAtLeast` from clang's runtime library.
/// rustc links without the C compiler's default libraries, so the runtime is
/// added here (it holds both arm64 and x86_64, for the universal build).
fn link_clang_runtime() {
    let macos = std::env::var("CARGO_CFG_TARGET_OS").is_ok_and(|os| os == "macos");
    if !macos || std::env::var_os("CARGO_FEATURE_TRANSCRIBE").is_none() {
        return;
    }
    let clang = |arg: &str| {
        let out = std::process::Command::new("xcrun").args(["clang", arg]).output().ok()?;
        let dir = String::from_utf8(out.stdout).ok()?.trim().to_string();
        (out.status.success() && !dir.is_empty()).then(|| std::path::PathBuf::from(dir))
    };
    let dir = [clang("--print-runtime-dir"), clang("-print-resource-dir").map(|d| d.join("lib/darwin"))]
        .into_iter()
        .flatten()
        .find(|d| d.join("libclang_rt.osx.a").is_file());
    match dir {
        Some(dir) => {
            println!("cargo:rustc-link-search=native={}", dir.display());
            println!("cargo:rustc-link-lib=static:-bundle=clang_rt.osx");
        }
        None => println!("cargo:warning=clang runtime (libclang_rt.osx.a) not found; the link may fail"),
    }
}
