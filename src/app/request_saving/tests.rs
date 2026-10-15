use super::PendingSave;
use crate::app::save_tests::{edited_request, saved_fixture, setup};
use crate::db::Database;
use crate::types::{HttpMethod, RequestData};
use gpui::TestAppContext;
use gpui_component::WindowExt as _;
use std::sync::Arc;

#[gpui::test]
fn save_and_close_keeps_edits_made_while_the_snapshot_is_saving(cx: &mut TestAppContext) {
    let db = Arc::new(Database::new_in_memory());
    let saved = saved_fixture(&db);
    let (app, cx) = setup(cx, db.clone());
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.open_saved_request_in_new_tab(&saved, window, cx);
            app.request_editor.update(cx, |editor, cx| {
                editor.load_request(&edited_request(), window, cx)
            });
            app.save_tab(0, false, true, window, cx);
            assert!(app.saving_tabs.contains(&0));
            app.request_editor.update(cx, |editor, cx| {
                editor.load_request(
                    &RequestData::new(HttpMethod::DELETE, "https://example.test/newer".into()),
                    window,
                    cx,
                )
            });
        })
    });
    cx.run_until_parked();
    assert_eq!(
        db.load_saved_request(saved.id)
            .unwrap()
            .unwrap()
            .request
            .method,
        HttpMethod::POST
    );
    cx.update(|window, cx| {
        assert!(!window.has_active_dialog(cx));
        let app = app.read(cx);
        assert_eq!(app.request_tabs[0].id, 0);
        assert_eq!(app.request_tabs[0].request.method, HttpMethod::DELETE);
        assert!(app.request_tabs[0].is_dirty());
    });
}

#[gpui::test]
fn saving_and_closing_an_inactive_tab_preserves_the_active_editor(cx: &mut TestAppContext) {
    let db = Arc::new(Database::new_in_memory());
    let saved = saved_fixture(&db);
    let (app, cx) = setup(cx, db.clone());
    let other = RequestData::new(HttpMethod::PATCH, "https://example.test/other".into());
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.open_saved_request_in_new_tab(&saved, window, cx);
            app.request_editor.update(cx, |editor, cx| {
                editor.load_request(&edited_request(), window, cx)
            });
            app.create_new_tab(window, cx);
            app.request_editor
                .update(cx, |editor, cx| editor.load_request(&other, window, cx));
            app.close_tab(0, window, cx);
        })
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(
        db.load_saved_request(saved.id)
            .unwrap()
            .unwrap()
            .request
            .method,
        HttpMethod::POST
    );
    cx.update(|window, cx| {
        assert!(!window.has_active_dialog(cx));
        let app = app.read(cx);
        assert_eq!(app.request_tabs.len(), 1);
        assert_eq!(app.request_tabs[0].id, 1);
        assert_eq!(
            app.request_editor.read(cx).get_current_request_data(cx).url,
            other.url
        );
        assert!(app.request_tabs[0].is_dirty());
    });
}

#[gpui::test]
fn failed_save_as_keeps_the_original_identity_and_baseline(cx: &mut TestAppContext) {
    let db = Arc::new(Database::new_in_memory());
    let saved = saved_fixture(&db);
    let (app, cx) = setup(cx, db.clone());
    let expected = cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.open_saved_request_in_new_tab(&saved, window, cx);
            app.request_editor.update(cx, |editor, cx| {
                editor.load_request(&edited_request(), window, cx)
            });
            app.save_current_tab_state(cx);
            let expected = app.request_tabs[0].snapshot();
            let mut snapshot = expected.clone();
            snapshot.collection_id = Some(-1);
            snapshot.folder_id = None;
            app.persist_saved_request(
                PendingSave {
                    tab_id: 0,
                    saved_id: None,
                    snapshot,
                    close_after: false,
                },
                window,
                cx,
            );
            expected
        })
    });
    cx.run_until_parked();
    assert_eq!(db.load_saved_request(saved.id).unwrap().unwrap(), saved);
    cx.update(|window, cx| {
        assert!(window.has_active_dialog(cx));
        let app = app.read(cx);
        assert_eq!(app.request_tabs[0].saved_request_id, Some(saved.id));
        assert_eq!(app.request_tabs[0].snapshot(), expected);
        assert!(app.request_tabs[0].is_dirty());
        assert!(app.saving_tabs.is_empty());
    });
}
