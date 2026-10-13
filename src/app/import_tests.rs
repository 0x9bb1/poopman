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
    let existing_id = db.create_environment("Existing").unwrap();
    db.replace_variables(
        existing_id,
        &[EnvVar {
            enabled: true,
            key: "token".into(),
            value: "disk-value".into(),
        }],
    )
    .unwrap();
    db.set_active_environment_id(Some(existing_id)).unwrap();
    let mut environments = db.load_environments().unwrap();
    environments[0].variables[0].value = "pending-edit".into();
    let imported = crate::postman::import_collection(include_str!(
        "../../tests/fixtures/import/postman-variables.json"
    ))
    .unwrap();
    let (_, environment) = db
        .insert_collection_import(&imported.collection, "Scoped API", &imported.variables)
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
    std::thread::spawn(move || db.register_ui_thread())
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
        cx.run_until_parked();
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
    cx.run_until_parked();
    cx.update(|_, cx| {
        let app = app.read(cx);
        assert_eq!(app.active_environment_id, Some(environment.id));
        let request = app.request_editor.read(cx).resolved_request_data(cx);
        assert_eq!(request.url, "https://example.test/items?count=10");
        assert_eq!(request.auth.api_key_value, "imported-token");
    });
}
