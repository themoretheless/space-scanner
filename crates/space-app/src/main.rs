mod anim;
mod app;
mod junk_panel;
mod largest_files;
mod review;
mod rules_panel;
mod settings;
mod theme;
mod tm_clean;
mod views;
mod volumes;
mod widgets;

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
            .with_min_inner_size([720.0, 520.0])
            .with_title("Space Scanner")
            .with_drag_and_drop(true),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };

    eframe::run_native(
        "Space Scanner",
        options,
        Box::new(move |cc| Ok(Box::new(SpaceApp::new(cc, settings)))),
    )
}
