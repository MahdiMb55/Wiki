#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

#[cfg(windows)]
mod actions;
#[cfg(windows)]
mod listening;
#[cfg(windows)]
mod speech;
#[cfg(windows)]
mod ui;
#[cfg(windows)]
mod vosk;

#[cfg(windows)]
fn main() {
    if let Err(error) = run() {
        actions::show_error(&format!("{error:#}"));
    }
}

#[cfg(windows)]
fn run() -> anyhow::Result<()> {
    use eframe::egui;

    let Some(_instance) = actions::Instance::take()? else {
        ui::reopen();
        return Ok(());
    };
    let start_hidden = std::env::args_os().any(|arg| arg == "--tray");
    let scale = 0.75;
    let options = eframe::NativeOptions {
        centered: true,
        viewport: egui::ViewportBuilder::default()
            .with_transparent(true)
            .with_inner_size(egui::vec2(1000., 680.) * scale)
            .with_min_inner_size(egui::vec2(570., 445.) * scale)
            // Let the embedded Windows icons take precedence over eframe's fallback.
            .with_icon(egui::IconData::default()),
        ..Default::default()
    };
    eframe::run_native(
        "Wiki",
        options,
        Box::new(move |cc| {
            cc.egui_ctx
                .options_mut(|options| options.zoom_factor = scale);
            Ok(Box::new(ui::Wiki::new(cc, start_hidden)?))
        }),
    )
    .map_err(|error| anyhow::anyhow!("Could not open Wiki: {error}"))
}

#[cfg(not(windows))]
fn main() {
    eprintln!("Wiki's desktop app requires Windows.");
}
