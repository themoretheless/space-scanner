mod anim;
mod app;
mod review;
mod settings;
mod side_tree;
mod theme;
mod tm_clean;
mod views;
mod volumes;

use app::SpaceApp;

fn main() -> eframe::Result<()> {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let msg = format!("{info}");
        let _ = std::fs::write("/tmp/space-scanner-panic.log", &msg);
        eprintln!("space-scanner panic: {msg}");
        default_hook(info);
    }));

    let settings = settings::AppSettings::load();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([settings.window_width, settings.window_height])
            .with_min_inner_size([900.0, 600.0])
            .with_title("Space Scanner"),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };

    eframe::run_native(
        "Space Scanner",
        options,
        Box::new(move |cc| Ok(Box::new(SpaceApp::new(cc, settings)))),
    )
}
