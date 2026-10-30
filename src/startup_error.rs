//! The window shown instead of the main UI when startup data cannot be loaded
//! (for example, the database cannot be opened or migrated). Without it the
//! process would run with no window and no explanation.

use gpui::*;
use gpui_component::{
    ActiveTheme as _, Root,
    button::{Button, ButtonVariants as _},
    h_flex, v_flex,
};

pub struct StartupErrorView {
    details: SharedString,
}

impl StartupErrorView {
    pub fn new(error: &anyhow::Error) -> Self {
        Self {
            // `{:#}` keeps the whole context chain, which names the failing path.
            details: format!("{error:#}").into(),
        }
    }
}

impl Render for StartupErrorView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let log_path = std::env::temp_dir().join("poopman").join("poopman.log");
        v_flex()
            .size_full()
            .bg(theme.background)
            .p_6()
            .gap_3()
            .child(
                div()
                    .text_lg()
                    .font_weight(FontWeight::BOLD)
                    .child("Poopman could not start"),
            )
            .child(
                div()
                    .debug_selector(|| "startup-error-details".into())
                    .text_sm()
                    .child(self.details.clone()),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(format!(
                        "Check that the data directory is writable and that no other \
                         Poopman window is running, then start Poopman again. \
                         Log: {}",
                        log_path.display()
                    )),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("startup-error-copy")
                            .ghost()
                            .label("Copy details")
                            .on_click(cx.listener(|this, _, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(
                                    this.details.to_string(),
                                ))
                            })),
                    )
                    .child(
                        Button::new("startup-error-quit")
                            .primary()
                            .label("Quit")
                            .on_click(|_, _, cx| cx.quit()),
                    ),
            )
    }
}

/// Open the startup error window. Closing it quits, since there is no other
/// window to return to.
pub fn open(error: &anyhow::Error, cx: &mut App) -> anyhow::Result<()> {
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::centered(size(px(560.), px(300.)), cx)),
        ..Default::default()
    };
    cx.open_window(options, |window, cx| {
        window.on_window_should_close(cx, |_, cx| {
            cx.quit();
            true
        });
        let view = cx.new(|_| StartupErrorView::new(error));
        cx.new(|cx| Root::new(view, window, cx))
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::StartupErrorView;
    use gpui::{AppContext as _, TestAppContext};
    use gpui_component::Root;

    #[gpui::test]
    fn renders_the_error_chain(cx: &mut TestAppContext) {
        cx.update(gpui_component::init);
        let error = anyhow::anyhow!("permission denied").context("open history.db");
        let (_, cx) = cx.add_window_view(|window, cx| {
            let view = cx.new(|_| StartupErrorView::new(&error));
            assert_eq!(
                view.read(cx).details.as_ref(),
                "open history.db: permission denied"
            );
            Root::new(view, window, cx)
        });
        cx.update(|window, cx| window.draw(cx).clear());
        assert!(cx.debug_bounds("startup-error-details").is_some());
    }
}
