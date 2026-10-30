use super::HistoryPanel;
use crate::db::Database;
use crate::types::{AuthConfig, BodyType};
use gpui::{AppContext as _, ClickEvent, TestAppContext};
use gpui_component::Root;
use std::sync::Arc;

#[gpui::test]
fn failed_clear_restores_rows_and_shows_a_toast(cx: &mut TestAppContext) {
    let db = Arc::new(Database::new_in_memory());
    db.insert_history(
        "GET",
        "https://api.example.test/items",
        "[]",
        &BodyType::None,
        &AuthConfig::default(),
    )
    .unwrap();
    // Reads keep working; only the clear's DELETE is rejected.
    db.execute_batch_for_test(
        "CREATE TRIGGER reject_history_delete BEFORE DELETE ON history
         BEGIN SELECT RAISE(ABORT, 'history is read-only'); END;",
    );
    let history = db.load_recent_history(super::HISTORY_LIMIT).unwrap();

    cx.update(gpui_component::init);
    let mut panel = None;
    let (_, cx) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| HistoryPanel::new(db.clone(), history, window, cx));
        panel = Some(view.clone());
        Root::new(view, window, cx)
    });
    let panel = panel.unwrap();

    cx.update(|window, cx| {
        panel.update(cx, |panel, cx| {
            panel.clear_history(&ClickEvent::default(), window, cx);
            assert!(panel.history.is_empty(), "the clear is optimistic");
        })
    });
    cx.run_until_parked();

    cx.update(|window, cx| {
        assert_eq!(panel.read(cx).history.len(), 1);
        let root = window.root::<Root>().flatten().unwrap();
        let toasts = root.read(cx).notification.read(cx).notifications();
        assert_eq!(toasts.len(), 1);
    });
}
