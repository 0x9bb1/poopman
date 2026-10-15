use super::save_tests::{edited_request, saved_fixture, setup};
use crate::db::Database;
use crate::types::HttpMethod;
use gpui::{Modifiers, TestAppContext, point, px, size};
use gpui_component::WindowExt as _;
use std::sync::Arc;

#[gpui::test]
fn save_button_updates_and_its_menu_saves_as_without_deleting(cx: &mut TestAppContext) {
    let db = Arc::new(Database::new_in_memory());
    let saved = saved_fixture(&db);
    let (app, cx) = setup(cx, db.clone());
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.open_saved_request_in_new_tab(&saved, window, cx);
            app.request_editor.update(cx, |editor, cx| {
                editor.load_request(&edited_request(), window, cx)
            });
        })
    });
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear());
    let bounds = cx.debug_bounds("request-save-actions").unwrap();
    let save_position = point(bounds.origin.x + px(20.), bounds.center().y);
    cx.simulate_mouse_move(save_position, None, Modifiers::none());
    cx.simulate_click(save_position, Modifiers::none());
    cx.run_until_parked();
    let updated = db.load_saved_request(saved.id).unwrap().unwrap();
    assert_eq!(updated.request.method, HttpMethod::POST);
    cx.update(|window, cx| {
        assert!(!window.has_active_dialog(cx));
        assert!(!app.read(cx).request_tabs[0].is_dirty());
        app.update(cx, |app, cx| {
            app.request_editor.update(cx, |editor, cx| {
                let mut request = edited_request();
                request.method = HttpMethod::PATCH;
                editor.load_request(&request, window, cx);
            })
        });
    });
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear());
    let bounds = cx.debug_bounds("request-save-actions").unwrap();
    let menu_position = point(bounds.right() - px(12.), bounds.center().y);
    cx.simulate_mouse_move(menu_position, None, Modifiers::none());
    cx.simulate_click(menu_position, Modifiers::none());
    cx.update(|window, cx| window.draw(cx).clear());
    // The menu opens below the trigger, with its right edge aligned to the arrow.
    let item_position = point(bounds.right() - px(48.), bounds.bottom() + px(20.));
    cx.simulate_mouse_move(item_position, None, Modifiers::none());
    cx.simulate_click(item_position, Modifiers::none());
    cx.update(|window, cx| assert!(window.has_active_dialog(cx)));
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    let id = cx.update(|_, cx| app.read(cx).request_tabs[0].saved_request_id.unwrap());
    assert_ne!(id, saved.id);
    assert_eq!(db.load_saved_request(saved.id).unwrap().unwrap(), updated);
    assert_eq!(
        db.load_saved_request(id).unwrap().unwrap().request.method,
        HttpMethod::PATCH
    );
}

#[gpui::test]
fn save_controls_and_close_confirmation_fit_the_minimum_window(cx: &mut TestAppContext) {
    let db = Arc::new(Database::new_in_memory());
    let (app, cx) = setup(cx, db);
    for width in [720., 1000., 1440.] {
        cx.simulate_resize(size(px(width), px(480.)));
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear());
        let viewport = cx.update(|window, _| window.viewport_size());
        let url = cx.debug_bounds("request-url-input").unwrap();
        let save = cx.debug_bounds("request-save-actions").unwrap();
        let send = cx.debug_bounds("request-send-actions").unwrap();
        assert!(
            url.size.width > px(40.),
            "URL input at width {width}: {url:?}"
        );
        assert!(
            save.size.width <= px(56.),
            "Save actions should remain compact: {save:?}"
        );
        assert_eq!(save.size.height, send.size.height);
        assert!(url.right() <= save.origin.x);
        assert!(save.right() <= send.origin.x);
        assert!(send.right() <= viewport.width);
    }
    cx.simulate_resize(size(px(720.), px(480.)));
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.request_editor.update(cx, |editor, cx| {
                editor.load_request(&edited_request(), window, cx)
            });
            app.request_tabs[0].saved_name = Some("Long request name ".repeat(40));
            app.close_tab(0, window, cx);
        })
    });
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear());
    let discard = cx.debug_bounds("discard-request-changes").unwrap();
    let viewport = cx.update(|window, _| window.viewport_size());
    assert!(discard.origin.x >= px(0.));
    assert!(discard.right() <= viewport.width);
    assert!(
        discard.bottom() <= viewport.height,
        "{discard:?} exceeds {viewport:?}"
    );
}
