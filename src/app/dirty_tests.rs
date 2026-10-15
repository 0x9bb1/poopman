use super::save_tests::{edited_request, saved_fixture, setup};
use crate::db::Database;
use crate::types::{
    AuthType, BodyDraft, BodyKind, BodyType, HistoryItem, HttpMethod, ParamState, RequestData,
};
use gpui::{Modifiers, TestAppContext};
use gpui_component::WindowExt as _;
use std::sync::Arc;

#[gpui::test]
fn partial_scratch_edits_survive_opening_history_and_collections(cx: &mut TestAppContext) {
    let db = Arc::new(Database::new_in_memory());
    let saved = saved_fixture(&db);
    let (app, cx) = setup(cx, db);
    for from_history in [true, false] {
        for field in 0..6 {
            cx.run_until_parked();
            cx.update(|_, cx| assert!(app.read(cx).request_tabs[0].is_blank()));
            let mut request = RequestData::new(HttpMethod::GET, String::new());
            request.body = BodyType::default();
            match field {
                0 => request.method = HttpMethod::POST,
                1 => {
                    request.auth.auth_type = AuthType::Bearer;
                    request.auth.bearer_token = "draft".into();
                }
                2 => request.headers.push(("X-Draft".into(), "kept".into())),
                3 => request.body = edited_request().body,
                _ => {}
            }
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    app.request_editor.update(cx, |editor, cx| {
                        editor.load_request(&request, window, cx);
                        if field == 4 {
                            editor.load_params_state(
                                &[ParamState {
                                    enabled: false,
                                    key: "draft".into(),
                                    value: "kept".into(),
                                }],
                                window,
                                cx,
                            );
                        }
                        if field == 5 {
                            let mut draft = BodyDraft::from_body(&request.body);
                            draft.formdata_rows.push(crate::types::FormDataRow {
                                enabled: false,
                                key: "draft".into(),
                                value: crate::types::FormDataValue::Text("kept".into()),
                            });
                            editor.load_request_with_body_draft(&request, Some(&draft), window, cx);
                        }
                    });
                })
            });
            cx.run_until_parked();
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    assert!(app.request_tabs[0].is_dirty(), "field {field}");
                    let expected = app.request_tabs[0].snapshot();
                    if from_history {
                        app.open_history_in_new_tab(
                            &HistoryItem::new(1, String::new(), saved.request.clone(), None),
                            window,
                            cx,
                        );
                    } else {
                        app.open_saved_request_in_new_tab(&saved, window, cx);
                    }
                    assert_eq!(app.request_tabs.len(), 2, "field {field}");
                    assert_eq!(app.request_tabs[0].snapshot(), expected);
                    assert!(!window.has_active_dialog(cx));
                    app.close_tab_without_prompt(1, window, cx);
                    app.close_tab_without_prompt(0, window, cx);
                })
            });
        }
    }
}

#[gpui::test]
fn loading_and_switching_saved_requests_does_not_create_false_dirty_state(cx: &mut TestAppContext) {
    let db = Arc::new(Database::new_in_memory());
    let saved = saved_fixture(&db);
    let (app, cx) = setup(cx, db);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.open_saved_request_in_new_tab(&saved, window, cx)
        })
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            assert!(!app.request_tabs[0].is_dirty());
            app.create_new_tab(window, cx);
            app.switch_to_tab(0, window, cx);
        })
    });
    cx.run_until_parked();
    cx.update(|_, cx| assert!(app.read(cx).request_tabs.iter().all(|tab| !tab.is_dirty())));
}

#[gpui::test]
fn close_cancel_and_discard_preserve_or_remove_the_target_tab(cx: &mut TestAppContext) {
    let db = Arc::new(Database::new_in_memory());
    let (app, cx) = setup(cx, db);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.request_editor.update(cx, |editor, cx| {
                editor.load_request(&edited_request(), window, cx)
            });
            app.create_new_tab(window, cx);
            app.close_tab(0, window, cx);
            assert_eq!(app.request_tabs.len(), 2);
            assert!(window.has_active_dialog(cx));
        })
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("escape");
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            assert!(!window.has_active_dialog(cx));
            assert_eq!(app.request_tabs.len(), 2);
            assert!(app.request_tabs[0].is_dirty());
            app.close_tab(0, window, cx);
        })
    });
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear());
    let bounds = cx
        .debug_bounds("discard-request-changes")
        .expect("discard button rendered");
    cx.simulate_mouse_move(bounds.center(), None, Modifiers::none());
    cx.simulate_click(bounds.center(), Modifiers::none());
    cx.run_until_parked();
    cx.update(|window, cx| {
        assert!(!window.has_active_dialog(cx));
        let app = app.read(cx);
        assert_eq!(app.request_tabs.len(), 1);
        assert_eq!(app.request_tabs[0].id, 1);
        assert!(app.request_tabs[0].is_blank());
    });
}

#[gpui::test]
fn save_before_close_updates_the_record_then_resets_the_last_tab(cx: &mut TestAppContext) {
    let db = Arc::new(Database::new_in_memory());
    let saved = saved_fixture(&db);
    let (app, cx) = setup(cx, db.clone());
    let closing_id = cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.open_saved_request_in_new_tab(&saved, window, cx);
            app.request_editor.update(cx, |editor, cx| {
                editor.load_request(&edited_request(), window, cx)
            });
            let id = app.request_tabs[0].id;
            app.close_tab(0, window, cx);
            assert_eq!(app.request_tabs[0].id, id);
            assert!(window.has_active_dialog(cx));
            id
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
            .body,
        edited_request().body
    );
    cx.update(|window, cx| {
        assert!(!window.has_active_dialog(cx));
        let app = app.read(cx);
        assert_ne!(app.request_tabs[0].id, closing_id);
        assert!(app.request_tabs[0].is_blank());
    });
}

#[gpui::test]
fn canceled_first_save_does_not_close_or_clean_the_tab(cx: &mut TestAppContext) {
    let db = Arc::new(Database::new_in_memory());
    let collection = db.create_collection("API").unwrap();
    let (app, cx) = setup(cx, db.clone());
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.request_editor.update(cx, |editor, cx| {
                editor.load_request(&edited_request(), window, cx)
            });
            app.close_tab(0, window, cx);
        })
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("enter");
    cx.update(|window, cx| assert!(window.has_active_dialog(cx)));
    cx.simulate_keystrokes("escape");
    cx.update(|window, cx| {
        assert!(!window.has_active_dialog(cx));
        assert!(app.read(cx).request_tabs[0].is_dirty());
        assert_eq!(app.read(cx).request_tabs[0].id, 0);
    });
    let collections = db.load_collections().unwrap();
    assert_eq!(collections[0].id, collection);
    assert!(collections[0].requests.is_empty());
}

#[gpui::test]
fn failed_save_before_close_keeps_the_tab_and_dirty_state(cx: &mut TestAppContext) {
    let db = Arc::new(Database::new_in_memory());
    let saved = saved_fixture(&db);
    let (app, cx) = setup(cx, db.clone());
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.open_saved_request_in_new_tab(&saved, window, cx);
            app.request_editor.update(cx, |editor, cx| {
                editor.load_request(&edited_request(), window, cx)
            });
            app.close_tab(0, window, cx);
        })
    });
    db.delete_saved_request(saved.id).unwrap();
    cx.run_until_parked();
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    cx.update(|window, cx| {
        assert!(window.has_active_dialog(cx));
        let app = app.read(cx);
        assert_eq!(app.request_tabs[0].saved_request_id, Some(saved.id));
        assert!(app.request_tabs[0].is_dirty());
        assert_eq!(app.request_tabs[0].request.body, edited_request().body);
    });
}

#[gpui::test]
fn save_as_creates_a_new_id_without_updating_the_original(cx: &mut TestAppContext) {
    let db = Arc::new(Database::new_in_memory());
    let saved = saved_fixture(&db);
    let (app, cx) = setup(cx, db.clone());
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.open_saved_request_in_new_tab(&saved, window, cx);
            app.request_editor.update(cx, |editor, cx| {
                editor.load_request(&edited_request(), window, cx)
            });
            app.save_active_request(true, window, cx);
        })
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    let new_id = cx.update(|window, cx| {
        assert!(!window.has_active_dialog(cx));
        let tab = &app.read(cx).request_tabs[0];
        assert!(!tab.is_dirty());
        tab.saved_request_id.unwrap()
    });
    assert_ne!(new_id, saved.id);
    assert_eq!(db.load_saved_request(saved.id).unwrap().unwrap(), saved);
    let copy = db.load_saved_request(new_id).unwrap().unwrap();
    assert_eq!(copy.request.body, edited_request().body);
    assert_eq!(copy.collection_id, saved.collection_id);
    assert_eq!(copy.folder_id, saved.folder_id);
}

#[gpui::test]
fn inactive_body_drafts_survive_save_reload_and_duplicate(cx: &mut TestAppContext) {
    let db = Arc::new(Database::new_in_memory());
    let saved = saved_fixture(&db);
    let (app, cx) = setup(cx, db.clone());
    let mut request = saved.request.clone();
    request.body = BodyType::None;
    let mut draft = BodyDraft::from_body(&request.body);
    draft.kind = BodyKind::None;
    draft.raw_content = "inactive raw content".into();
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.open_saved_request_in_new_tab(&saved, window, cx);
            app.request_editor.update(cx, |editor, cx| {
                editor.load_request_with_body_draft(&request, Some(&draft), window, cx)
            });
            app.save_request(window, cx);
        })
    });
    cx.run_until_parked();
    let updated = db.load_saved_request(saved.id).unwrap().unwrap();
    assert_eq!(updated.body_draft, Some(draft.clone()));
    assert_eq!(updated.request.body, BodyType::None);
    let duplicate = db.duplicate_saved_request(saved.id).unwrap();
    assert_eq!(
        db.load_saved_request(duplicate)
            .unwrap()
            .unwrap()
            .body_draft,
        Some(draft.clone())
    );
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            assert!(!app.request_tabs[0].is_dirty());
            app.close_tab(0, window, cx);
            assert!(!window.has_active_dialog(cx));
            app.open_saved_request_in_new_tab(&updated, window, cx);
            assert_eq!(app.request_editor.read(cx).get_body_draft(cx), draft);
        })
    });
}

#[gpui::test]
fn typing_and_reverting_body_updates_the_visible_dirty_indicator(cx: &mut TestAppContext) {
    let db = Arc::new(Database::new_in_memory());
    let saved = saved_fixture(&db);
    let (app, cx) = setup(cx, db);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.open_saved_request_in_new_tab(&saved, window, cx);
            app.request_editor
                .update(cx, |editor, cx| editor.focus_body_for_save_test(window, cx));
        })
    });
    cx.run_until_parked();
    cx.simulate_input("draft");
    cx.refresh().unwrap();
    cx.update(|_, cx| assert!(app.read(cx).request_tabs[0].is_dirty()));
    assert!(cx.debug_bounds("unsaved-request-tab").is_some());
    cx.simulate_keystrokes(if cfg!(target_os = "macos") {
        "cmd-a backspace"
    } else {
        "ctrl-a backspace"
    });
    cx.refresh().unwrap();
    cx.update(|_, cx| assert!(!app.read(cx).request_tabs[0].is_dirty()));
    // GPUI keeps debug_bounds entries from older frames, so absence cannot
    // be asserted through that map; the renderer uses the current dirty state.
}

#[gpui::test]
fn failed_first_save_preserves_the_unsaved_tab(cx: &mut TestAppContext) {
    let db = Arc::new(Database::new_in_memory());
    let collection = db.create_collection("API").unwrap();
    let (app, cx) = setup(cx, db.clone());
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.request_editor.update(cx, |editor, cx| {
                editor.load_request(&edited_request(), window, cx)
            });
            app.save_request(window, cx);
        })
    });
    cx.run_until_parked();
    db.delete_collection(collection).unwrap();
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    cx.update(|window, cx| {
        assert!(window.has_active_dialog(cx));
        let app = app.read(cx);
        assert!(app.request_tabs[0].is_dirty());
        assert_eq!(app.request_tabs[0].saved_request_id, None);
        assert_eq!(app.request_tabs[0].request.body, edited_request().body);
        assert!(app.saving_tabs.is_empty());
    });
}

#[gpui::test]
fn update_persists_name_and_destination_without_changing_id(cx: &mut TestAppContext) {
    let db = Arc::new(Database::new_in_memory());
    let saved = saved_fixture(&db);
    let destination = db.create_collection("Other").unwrap();
    let folder = db.create_folder(destination, None, "Nested").unwrap();
    let (app, cx) = setup(cx, db.clone());
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.open_saved_request_in_new_tab(&saved, window, cx);
            let tab = &mut app.request_tabs[0];
            tab.saved_name = Some("Renamed".into());
            tab.collection_id = Some(destination);
            tab.folder_id = Some(folder);
            app.save_request(window, cx);
        })
    });
    cx.run_until_parked();
    let updated = db.load_saved_request(saved.id).unwrap().unwrap();
    assert_eq!(updated.name, "Renamed");
    assert_eq!(updated.collection_id, destination);
    assert_eq!(updated.folder_id, Some(folder));
    assert_eq!(updated.created_at, saved.created_at);
    cx.update(|_, cx| assert!(!app.read(cx).request_tabs[0].is_dirty()));
}
