#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod i18n;
mod preferences;
mod theme;
mod ui;
mod worker;

use misf_core::{
    audit::{Action, AuditLog, Outcome},
    runtime::RuntimePaths,
};

fn main() -> eframe::Result {
    if std::env::args_os()
        .skip(1)
        .any(|arg| arg == "--help" || arg == "-h")
    {
        println!(
            "MISF Management Terminal\n  --debug.dir=<directory>  Isolated profile; loopback SSH only, real Passkey disabled\n  --debug.allow-passkey    Opt in to real authenticator operations in that debug profile"
        );
        return Ok(());
    }
    let (paths, audit) = match startup() {
        Ok(config) => config,
        Err(error) => {
            eprintln!("MISF startup failed: {error}");
            rfd::MessageDialog::new()
                .set_title("MISF startup failed")
                .set_description(&error)
                .set_level(rfd::MessageLevel::Error)
                .show();
            std::process::exit(2);
        }
    };
    let icon = eframe::icon_data::from_png_bytes(include_bytes!("../../assets/icons/misf-256.png"))
        .expect("The bundled MISF icon must be a valid PNG");
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_app_id("org.sparxilium.misf")
            .with_icon(icon)
            .with_title(if paths.debug {
                "MISF · DEBUG · Isolated profile"
            } else {
                "MISF · Management Terminal"
            })
            .with_inner_size([1280.0, 820.0])
            .with_min_inner_size([1024.0, 700.0]),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native(
        "MISF",
        options,
        Box::new(move |cc| Ok(Box::new(ui::ManagementApp::new(cc, paths, audit)))),
    )
}

fn startup() -> Result<(RuntimePaths, AuditLog), String> {
    let paths =
        RuntimePaths::from_args(std::env::args_os().skip(1)).map_err(|error| error.to_string())?;
    let audit = AuditLog::new(paths.store.parent().ok_or("Invalid data directory")?)
        .map_err(|error| error.to_string())?;
    audit
        .record(Action::AppStart, Outcome::Succeeded, None, None)
        .map_err(|error| error.to_string())?;
    if paths.debug_allow_passkey {
        audit
            .record(Action::DebugPasskeyEnabled, Outcome::Succeeded, None, None)
            .map_err(|error| error.to_string())?;
    }
    Ok((paths, audit))
}
