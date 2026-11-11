//! App shortcuts, driven through the production binding table.
//!
//! `secondary-` resolves per platform when the keystroke is parsed, so these
//! exercise Ctrl on Windows/Linux and Cmd on macOS — whichever the tests run on.

use super::PoopmanApp;
use super::save_tests::setup;
use crate::db::Database;
use crate::types::{AuthConfig, BodyType, HttpMethod, RequestData};
use gpui::{Entity, VisualTestContext};
use std::sync::Arc;

fn tab_count(app: &Entity<PoopmanApp>, cx: &mut VisualTestContext) -> usize {
    cx.update(|_, cx| app.read(cx).request_tabs.len())
}

fn active_tab(app: &Entity<PoopmanApp>, cx: &mut VisualTestContext) -> usize {
    cx.update(|_, cx| app.read(cx).active_tab_index)
}

fn url(app: &Entity<PoopmanApp>, cx: &mut VisualTestContext) -> String {
    cx.update(|_, cx| {
        app.read(cx)
            .request_editor
            .read(cx)
            .get_current_request_data(cx)
            .url
    })
}

/// A URL that fails before any network I/O, so sending is observable offline.
fn load_unsendable_url(app: &Entity<PoopmanApp>, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.request_editor.update(cx, |editor, cx| {
                let request = RequestData {
                    method: HttpMethod::GET,
                    url: "http://".into(),
                    headers: vec![],
                    body: BodyType::None,
                    auth: AuthConfig::default(),
                };
                editor.load_request(&request, window, cx);
            })
        })
    });
    cx.run_until_parked();
}

fn has_response(app: &Entity<PoopmanApp>, cx: &mut VisualTestContext) -> bool {
    cx.update(|_, cx| app.read(cx).request_tabs[0].response.is_some())
}

#[gpui::test]
fn new_tab_and_close_tab_shortcuts(cx: &mut gpui::TestAppContext) {
    let (app, cx) = setup(cx, Arc::new(Database::new_in_memory()));
    assert_eq!(tab_count(&app, cx), 1);

    cx.simulate_keystrokes("secondary-t");
    assert_eq!(tab_count(&app, cx), 2);
    assert_eq!(active_tab(&app, cx), 1);

    cx.simulate_keystrokes("secondary-w");
    assert_eq!(tab_count(&app, cx), 1);
    assert_eq!(active_tab(&app, cx), 0);
}

#[gpui::test]
fn every_tab_switching_shortcut_cycles_tabs(cx: &mut gpui::TestAppContext) {
    let (app, cx) = setup(cx, Arc::new(Database::new_in_memory()));
    cx.simulate_keystrokes("secondary-t secondary-t");
    assert_eq!(tab_count(&app, cx), 3);
    assert_eq!(active_tab(&app, cx), 2);

    // Each spelling of "next" from the last tab wraps to the first; the paired
    // spelling of "previous" wraps back.
    for (next, prev) in [
        ("secondary-shift-]", "secondary-shift-["),
        ("secondary-}", "secondary-{"),
        ("ctrl-tab", "ctrl-shift-tab"),
    ] {
        cx.simulate_keystrokes(next);
        assert_eq!(active_tab(&app, cx), 0, "{next} should select the next tab");
        cx.simulate_keystrokes(prev);
        assert_eq!(
            active_tab(&app, cx),
            2,
            "{prev} should select the previous tab"
        );
    }
}

#[gpui::test]
fn focus_url_shortcut_selects_the_url_for_replacement(cx: &mut gpui::TestAppContext) {
    let (app, cx) = setup(cx, Arc::new(Database::new_in_memory()));
    load_unsendable_url(&app, cx);

    cx.simulate_keystrokes("secondary-l");
    cx.simulate_input("https://typed.test");
    cx.run_until_parked();

    assert_eq!(url(&app, cx), "https://typed.test");
}

#[gpui::test]
fn send_shortcut_works_from_the_app_root(cx: &mut gpui::TestAppContext) {
    // Focus on the app root: the context-free binding.
    let (app, cx) = setup(cx, Arc::new(Database::new_in_memory()));
    load_unsendable_url(&app, cx);
    assert!(!has_response(&app, cx));
    cx.simulate_keystrokes("secondary-enter");
    cx.run_until_parked();
    assert!(has_response(&app, cx), "send from the app root");
}

#[gpui::test]
fn send_shortcut_wins_over_the_inputs_own_binding(cx: &mut gpui::TestAppContext) {
    // Focus in the URL input: gpui-component binds the same keystroke in its
    // "Input" context, and ours must shadow it.
    let (app, cx) = setup(cx, Arc::new(Database::new_in_memory()));
    load_unsendable_url(&app, cx);
    cx.simulate_keystrokes("secondary-l");
    assert!(!has_response(&app, cx));
    cx.simulate_keystrokes("secondary-enter");
    cx.run_until_parked();
    assert!(has_response(&app, cx), "send from inside an input");
}

/// Whatever holds focus can vanish with the element that owned it. The
/// window's focus path is then empty, and no action reaches the app root.
#[gpui::test]
fn shortcuts_survive_the_window_losing_focus(cx: &mut gpui::TestAppContext) {
    let (app, cx) = setup(cx, Arc::new(Database::new_in_memory()));
    cx.simulate_keystrokes("secondary-t");
    assert_eq!(tab_count(&app, cx), 2);

    cx.update(|window, _| window.blur());
    cx.run_until_parked();

    cx.simulate_keystrokes("secondary-t");
    assert_eq!(tab_count(&app, cx), 3);
}
