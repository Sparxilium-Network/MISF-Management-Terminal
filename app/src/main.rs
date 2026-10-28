#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod ui;
mod worker;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("MISF · Management Terminal")
            .with_inner_size([1280.0, 820.0])
            .with_min_inner_size([1024.0, 700.0]),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native(
        "MISF",
        options,
        Box::new(|cc| Ok(Box::new(ui::ManagementApp::new(cc)))),
    )
}
