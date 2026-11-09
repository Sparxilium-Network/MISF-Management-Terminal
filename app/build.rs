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
