mod app;
mod review;
mod theme;
mod views;
mod volumes;

use app::SpaceApp;

fn main() -> eframe::Result<()> {
    // Faster cold start: skip vsync wait on first frames isn't exposed; keep window simple.
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([900.0, 600.0])
            .with_title("Space Scanner"),
        ..Default::default()
    };

    eframe::run_native(
        "Space Scanner",
        options,
        Box::new(|cc| {
            // Show UI immediately; heavy disk enumeration happens in background threads.
            Ok(Box::new(SpaceApp::new(cc)))
        }),
    )
}
