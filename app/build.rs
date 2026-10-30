fn main() {
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
