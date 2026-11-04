#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod audio;
mod config;
mod docx;
mod generate;
mod logger;
mod player;
mod text;
mod theme;
mod tls;
mod translate;
mod tts;
mod voices;

/// Icona della finestra: 128x128 RGBA incorporata.
fn app_icon() -> eframe::egui::IconData {
    eframe::egui::IconData {
        rgba: include_bytes!("../assets/icon.rgba").to_vec(),
        width: 128,
        height: 128,
    }
}

fn main() -> eframe::Result {
    tls::install_crypto_provider();
    let (w, h) = app::App::initial_window_size();
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title(format!("{} v{}", app::APP_TITLE, app::APP_VERSION))
            .with_inner_size([w, h])
            .with_min_inner_size([1100.0, 700.0])
            .with_icon(app_icon()),
        ..Default::default()
    };
    eframe::run_native(
        app::APP_TITLE,
        options,
        Box::new(|cc| Ok(Box::new(app::App::new(cc)))),
    )
}
