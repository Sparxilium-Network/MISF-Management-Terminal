fn main() {
    // The fluent style on every platform, so the app looks the same everywhere. Debug
    // builds keep element names so tests can find elements; release builds leave them out.
    slint_build::compile_with_config(
        "ui/main.slint",
        slint_build::CompilerConfiguration::new()
            .with_style("fluent".into())
            .with_debug_info(std::env::var("PROFILE").as_deref() == Ok("debug")),
    )
    .expect("Failed to compile the Slint UI");
    println!("cargo:rerun-if-changed=../assets/icons/misf.ico");
    println!("cargo:rerun-if-changed=Cargo.toml");
    println!("cargo:rustc-env=MISF_BUILD_NAME={}", build_name());

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winresource::WindowsResource::new()
            .set_icon("../assets/icons/misf.ico")
            .set("ProductName", "MISF Management Terminal")
            .set("FileDescription", "MISF Management Terminal")
            .set("InternalName", "misf")
            .set("OriginalFilename", "misf.exe")
            .compile()
            .expect("Failed to embed the MISF Windows icon and version resources");
    }
}

/// The name shown in Settings and the session log. A GitHub Actions build gets the same
/// name as its release (see the publish step in .github/workflows/build-prerelease.yml):
/// the tag for a `v*` tag push, otherwise `dev-<run>.<attempt>-<short commit>`. Local
/// builds show the crate version.
fn build_name() -> String {
    let var = |name: &str| {
        println!("cargo:rerun-if-env-changed={name}");
        std::env::var(name).unwrap_or_default()
    };
    let (actions, ref_type, ref_name) = (
        var("GITHUB_ACTIONS"),
        var("GITHUB_REF_TYPE"),
        var("GITHUB_REF_NAME"),
    );
    let (run, attempt, sha) = (
        var("GITHUB_RUN_NUMBER"),
        var("GITHUB_RUN_ATTEMPT"),
        var("GITHUB_SHA"),
    );
    if actions == "true" {
        if ref_type == "tag" && !ref_name.is_empty() {
            return ref_name;
        }
        if !run.is_empty() && !attempt.is_empty() && sha.len() >= 7 {
            return format!("dev-{run}.{attempt}-{}", &sha[..7]);
        }
    }
    format!(
        "v{}",
        std::env::var("CARGO_PKG_VERSION").unwrap_or_default()
    )
}
