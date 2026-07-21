#![windows_subsystem = "windows"]

mod app;
mod body_editor;
mod code_formatter;
mod code_gen;
mod code_snippet_panel;
mod curl_import;
mod db;
mod environment_manager;
mod format;
mod header_completion;
mod header_names;
mod history_panel;
mod http_client;
mod menu_bar;
mod request_editor;
mod request_tab;
mod response_viewer;
mod tab_bar;
mod theme;
mod types;
mod ui;
mod variables;
mod url_params;

use gpui::*;
use gpui_component::Root;
use rust_embed::RustEmbed;
use std::borrow::Cow;
use std::fs::OpenOptions;
use std::io::Write;

use crate::app::PoopmanApp;

/// An asset source that loads assets from the `./assets` folder.
#[derive(RustEmbed)]
#[folder = "./assets"]
#[include = "icons/**/*.svg"]
pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if path.is_empty() {
            return Ok(None);
        }

        match Self::get(path) {
            Some(f) => Ok(Some(f.data)),
            None => {
                // Don't error for missing assets - gpui-component may request icons we don't have
                log::debug!("Asset not found: {}", path);
                Ok(None)
            }
        }
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Ok(Self::iter()
            .filter_map(|p| p.starts_with(path).then(|| p.into()))
            .collect())
    }
}

/// Setup logger to write to both console and file
fn setup_logger() {
    // Use system temp directory for logs
    let log_dir = std::env::temp_dir().join("poopman");

    // Create poopman directory in temp if it doesn't exist
    std::fs::create_dir_all(&log_dir).expect("Failed to create log directory");

    let log_file_path = log_dir.join("poopman.log");

    // Open log file in append mode
    let log_file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_file_path)
        .expect("Failed to open log file");

    // Clone file handle for the builder
    let file_clone = log_file.try_clone().expect("Failed to clone file handle");

    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format(move |_buf, record| {
            let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");

            // Write to file only (not console)
            let mut file = file_clone.try_clone().expect("Failed to clone file");
            writeln!(
                file,
                "[{} {:5}] {}",
                timestamp,
                record.level(),
                record.args()
            ).ok();

            Ok(())
        })
        .init();

    log::info!("Poopman started - logging to: {}", log_file_path.display());
}

fn main() {
    // Initialize logger to write to both console and file
    setup_logger();

    let app = Application::new().with_assets(Assets);

    app.run(move |cx| {
        gpui_component::init(cx);
        crate::theme::apply_theme(cx);

        // macOS routes Cmd+Q through the application menu bar, so a keybinding
        // alone is not enough: without a menu item bound to this action the
        // keystroke has nowhere to dispatch and the app cannot be quit. Register
        // the handler (which also makes the menu item enabled — gpui gates that on
        // the action being available), bind the key so the menu item carries the
        // Cmd+Q equivalent, and install the menu below. `set_menus` is a harmless
        // no-op on Windows/Linux, which quit via the window's close button.
        cx.on_action(|_: &crate::app::Quit, cx| cx.quit());

        // Late binding on purpose: gpui gives later-added bindings precedence,
        // so the "Input"-context ctrl-enter shadows gpui-component's own
        // secondary-enter input binding (its PressEnter{secondary:true} event
        // is unused in this app).
        cx.bind_keys([
            KeyBinding::new("ctrl-enter", crate::app::SendRequest, None),
            KeyBinding::new("ctrl-enter", crate::app::SendRequest, Some("Input")),
            KeyBinding::new("ctrl-t", crate::app::NewTab, None),
            KeyBinding::new("ctrl-w", crate::app::CloseTab, None),
            KeyBinding::new("ctrl-tab", crate::app::NextTab, None),
            KeyBinding::new("ctrl-shift-tab", crate::app::PrevTab, None),
            KeyBinding::new("ctrl-l", crate::app::FocusUrl, None),
            KeyBinding::new("cmd-q", crate::app::Quit, None),
        ]);

        cx.set_menus(vec![Menu {
            name: "Poopman".into(),
            items: vec![MenuItem::action("Quit Poopman", crate::app::Quit)],
        }]);

        cx.spawn(async move |cx| {
            let window_options = WindowOptions {
                titlebar: Some(gpui_component::TitleBar::title_bar_options()),
                window_min_size: Some(size(px(720.), px(480.))),
                ..Default::default()
            };
            cx.open_window(window_options, |window, cx| {
                let view = cx.new(|cx| PoopmanApp::new(window, cx));
                cx.new(|cx| Root::new(view, window, cx))
            })?;
            Ok::<_, anyhow::Error>(())
        })
        .detach();
    });
}
