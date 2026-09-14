use super::{AppInitialState, PoopmanApp, SAVE_REQUEST_KEY, bind_save_request_shortcut};
use crate::db::Database;
use crate::environment_persistence::EnvironmentPersistence;
use crate::types::{
    AppSettings, AuthConfig, AuthType, BodyType, HeaderState, HeaderType, HttpMethod, ParamState,
    RawSubtype, RequestData, SavedRequest,
};
use gpui::{AppContext as _, Entity, TestAppContext, VisualTestContext};
use gpui_component::{Root, WindowExt as _};
use std::sync::Arc;

fn setup(
    cx: &mut TestAppContext,
    db: Arc<Database>,
) -> (Entity<PoopmanApp>, &mut VisualTestContext) {
    let initial = AppInitialState {
        collections: db.load_collections().unwrap(),
        environment_persistence: EnvironmentPersistence::new(db.clone()),
        db: db.clone(),
        environments: Vec::new(),
        active_environment_id: None,
        history: Vec::new(),
        settings: AppSettings::default(),
    };
    // GPUI's deterministic test executor runs background futures on the test
    // thread too. Reserve a different thread ID for the DB's UI-thread guard;
    // production guard behavior has its own database regression test.
    std::thread::spawn(move || db.register_ui_thread())
        .join()
        .unwrap();
    cx.update(gpui_component::init);
    cx.update(bind_save_request_shortcut);
    let mut app = None;
    let (_, cx) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| PoopmanApp::new(initial, window, cx));
        app = Some(view.clone());
        Root::new(view, window, cx)
    });
    (app.unwrap(), cx)
}

fn saved_fixture(db: &Database) -> SavedRequest {
    let collection_id = db.create_collection("API").unwrap();
    let folder_id = db.create_folder(collection_id, None, "Folder").unwrap();
    let request = RequestData::new(HttpMethod::GET, "https://example.test/original".into());
    let id = db
        .insert_saved_request(
            collection_id,
            Some(folder_id),
            "Keep this name",
            &request,
            &[],
            &[],
        )
        .unwrap();
    db.load_saved_request(id).unwrap().unwrap()
}

fn edited_request() -> RequestData {
    RequestData {
        method: HttpMethod::POST,
        url: "https://example.test/edited?q=1".into(),
        headers: vec![],
        body: BodyType::Raw {
            content: "{\"changed\":true}".into(),
            subtype: RawSubtype::Json,
        },
        auth: AuthConfig {
            auth_type: AuthType::Bearer,
            bearer_token: "{{token}}".into(),
            ..Default::default()
        },
    }
}

#[gpui::test]
fn shortcut_updates_original_record_from_url_and_body_inputs(cx: &mut TestAppContext) {
    let db = Arc::new(Database::new_in_memory());
    let saved = saved_fixture(&db);
    let (app, cx) = setup(cx, db.clone());
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.open_saved_request_in_new_tab(&saved, window, cx)
        })
    });
    for from_body in [false, true] {
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.request_editor.update(cx, |editor, cx| {
                    let mut request = edited_request();
                    if from_body {
                        request.method = HttpMethod::PATCH;
                    }
                    editor.load_request(&request, window, cx);
                });
            });
        });
        // Finish URL loading before editing disabled rows or changing focus,
        // as a user does when opening a request and then editing its fields.
        cx.run_until_parked();
        let expected = cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.request_editor.update(cx, |editor, cx| {
                    editor.load_params_state(
                        &[
                            ParamState {
                                enabled: true,
                                key: "q".into(),
                                value: "1".into(),
                            },
                            ParamState {
                                enabled: false,
                                key: "disabled".into(),
                                value: "kept".into(),
                            },
                        ],
                        window,
                        cx,
                    );
                    editor.load_headers_state(
                        &[HeaderState {
                            enabled: false,
                            key: "X-Disabled".into(),
                            value: "kept".into(),
                            header_type: HeaderType::Custom,
                            predefined: None,
                        }],
                        window,
                        cx,
                    );
                    if from_body {
                        editor.focus_body_for_save_test(window, cx);
                    } else {
                        editor.focus_url(window, cx);
                    }
                    (
                        editor.get_current_request_data(cx),
                        editor.get_params_state(cx),
                        editor.get_headers_state(cx),
                    )
                })
            })
        });
        cx.run_until_parked();
        cx.simulate_keystrokes(SAVE_REQUEST_KEY);
        cx.run_until_parked();
        let updated = db.load_saved_request(saved.id).unwrap().unwrap();
        assert_eq!(updated.name, saved.name);
        assert_eq!(updated.collection_id, saved.collection_id);
        assert_eq!(updated.folder_id, saved.folder_id);
        assert_eq!(updated.created_at, saved.created_at);
        assert_eq!(updated.request, expected.0);
        assert_eq!(updated.params_state, expected.1);
        assert_eq!(updated.headers_state, expected.2);
        assert_eq!(
            db.load_collections().unwrap()[0].folders[0].requests.len(),
            1
        );
        cx.update(|window, cx| {
            assert!(!window.has_active_dialog(cx));
            assert!(app.read(cx).saving_tabs.is_empty());
        });
    }
}

#[gpui::test]
fn unsaved_shortcut_uses_existing_dialog_once_and_inserts(cx: &mut TestAppContext) {
    let db = Arc::new(Database::new_in_memory());
    let (app, cx) = setup(cx, db.clone());
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.request_editor.update(cx, |editor, cx| {
                editor.load_request(&edited_request(), window, cx);
                editor.focus_url(window, cx);
            });
        })
    });
    cx.run_until_parked();
    cx.simulate_keystrokes(SAVE_REQUEST_KEY);
    cx.run_until_parked();
    assert_eq!(db.load_collections().unwrap().len(), 1);
    assert!(db.load_collections().unwrap()[0].requests.is_empty());
    cx.update(|window, cx| {
        assert!(window.has_active_dialog(cx));
        app.update(cx, |app, cx| {
            app.save_request(window, cx);
            app.save_request(window, cx);
        });
        window.close_dialog(cx);
        assert!(
            !window.has_active_dialog(cx),
            "shortcut must not stack dialogs"
        );
    });
    cx.update(|window, cx| app.update(cx, |app, cx| app.save_request(window, cx)));
    cx.run_until_parked();
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    let collections = db.load_collections().unwrap();
    assert_eq!(collections.len(), 1);
    assert_eq!(collections[0].requests.len(), 1);
    let inserted = &collections[0].requests[0];
    assert_eq!(inserted.request.method, HttpMethod::POST);
    assert_eq!(inserted.request.body, edited_request().body);
    assert_eq!(inserted.request.auth, edited_request().auth);
    cx.update(|window, cx| {
        assert!(!window.has_active_dialog(cx));
        assert_eq!(
            app.read(cx).request_tabs[0].saved_request_id,
            Some(inserted.id)
        );
    });
}

#[gpui::test]
fn failed_update_preserves_edits_and_reports_error(cx: &mut TestAppContext) {
    let db = Arc::new(Database::new_in_memory());
    let saved = saved_fixture(&db);
    let (app, cx) = setup(cx, db.clone());
    let expected = cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.open_saved_request_in_new_tab(&saved, window, cx);
            app.request_editor.update(cx, |editor, cx| {
                editor.load_request(&edited_request(), window, cx);
                editor.get_current_request_data(cx)
            })
        })
    });
    db.delete_saved_request(saved.id).unwrap();
    cx.update(|window, cx| app.update(cx, |app, cx| app.save_request(window, cx)));
    cx.run_until_parked();
    cx.update(|window, cx| {
        assert!(window.has_active_dialog(cx));
        let app = app.read(cx);
        assert!(app.saving_tabs.is_empty());
        assert_eq!(app.request_tabs[0].saved_request_id, Some(saved.id));
        assert_eq!(
            app.request_editor.read(cx).get_current_request_data(cx),
            expected
        );
    });
    assert!(db.load_saved_request(saved.id).unwrap().is_none());
}

#[gpui::test]
fn pending_save_keeps_its_snapshot_and_originating_tab(cx: &mut TestAppContext) {
    let db = Arc::new(Database::new_in_memory());
    let saved = saved_fixture(&db);
    let (app, cx) = setup(cx, db.clone());
    let expected = cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.open_saved_request_in_new_tab(&saved, window, cx);
            app.request_editor.update(cx, |editor, cx| {
                editor.load_request(&edited_request(), window, cx)
            });
            let snapshot = app.request_editor.read(cx).get_current_request_data(cx);
            app.save_request(window, cx);
            assert_eq!(app.saving_tabs.len(), 1);
            app.request_editor.update(cx, |editor, cx| {
                editor.load_request(
                    &RequestData::new(HttpMethod::DELETE, "https://example.test/later".into()),
                    window,
                    cx,
                )
            });
            app.save_request(window, cx);
            assert_eq!(app.saving_tabs.len(), 1);
            app.create_new_tab(window, cx);
            snapshot
        })
    });
    cx.run_until_parked();
    assert_eq!(
        db.load_saved_request(saved.id).unwrap().unwrap().request,
        expected
    );
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            assert_eq!(app.active_tab_index, 1);
            assert!(app.saving_tabs.is_empty());
            app.switch_to_tab(0, window, cx);
            assert_eq!(
                app.request_editor
                    .read(cx)
                    .get_current_request_data(cx)
                    .method,
                HttpMethod::DELETE
            );
        })
    });
}
