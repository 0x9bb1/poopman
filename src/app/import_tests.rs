use super::{AppInitialState, PoopmanApp};
use crate::collections_panel::CollectionsChanged;
use crate::db::Database;
use crate::environment_persistence::EnvironmentPersistence;
use crate::types::{AppSettings, EnvVar};
use gpui::{AppContext as _, TestAppContext};
use gpui_component::Root;
use std::sync::Arc;

#[gpui::test]
fn imported_environment_is_selectable_without_reloading_pending_edits(cx: &mut TestAppContext) {
    let db = Arc::new(Database::new_in_memory());
    let existing_id = db.create_environment("Existing").wait().unwrap();
    db.replace_variables(
        existing_id,
        &[EnvVar {
            enabled: true,
            key: "token".into(),
            value: "disk-value".into(),
        }],
    )
    .wait()
    .unwrap();
    db.set_active_environment_id(Some(existing_id))
        .wait()
        .unwrap();
    let mut environments = db.load_environments().wait().unwrap();
    environments[0].variables[0].value = "pending-edit".into();
    let imported = crate::postman::import_collection(include_str!(
        "../../tests/fixtures/import/postman-variables.json"
    ))
    .unwrap();
    let (_, environment) = db
        .insert_collection_import(&imported.collection, "Scoped API", &imported.variables)
        .wait()
        .unwrap();
    let environment = environment.unwrap();
    let initial = AppInitialState {
        db: db.clone(),
        environment_persistence: EnvironmentPersistence::new(db.clone()),
        environments,
        active_environment_id: Some(existing_id),
        collections: Vec::new(),
        history: Vec::new(),
        settings: AppSettings::default(),
    };
    let other_thread_db = db.clone();
    std::thread::spawn(move || other_thread_db.register_ui_thread())
        .join()
        .unwrap();
    cx.update(gpui_component::init);
    let mut app = None;
    let (_, cx) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| PoopmanApp::new(initial, window, cx));
        app = Some(view.clone());
        Root::new(view, window, cx)
    });
    let app = app.unwrap();
    for _ in 0..2 {
        cx.update(|_, cx| {
            let panel = app.read(cx).collections_panel.clone();
            panel.update(cx, |_, cx| {
                cx.emit(CollectionsChanged {
                    imported_environment: Some(environment.clone()),
                    ..Default::default()
                })
            });
        });
        crate::test_support::settle(cx, &db);
    }
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            assert_eq!(app.environments.len(), 2);
            assert_eq!(app.environments[0].variables[0].value, "pending-edit");
            assert_eq!(app.active_environment_id, Some(existing_id));
            app.request_editor.update(cx, |editor, cx| {
                editor.load_request(&imported.collection.requests[0].request, window, cx);
            });
            app.set_active_environment(Some(environment.id), window, cx);
        });
    });
    crate::test_support::settle(cx, &db);
    cx.update(|_, cx| {
        let app = app.read(cx);
        assert_eq!(app.active_environment_id, Some(environment.id));
        let request = app.request_editor.read(cx).resolved_request_data(cx);
        assert_eq!(request.url, "https://example.test/items?count=10");
        assert_eq!(request.auth.api_key_value, "imported-token");
    });
}
