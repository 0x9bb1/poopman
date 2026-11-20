use super::{HistoryItemClicked, HistoryPanel};
use crate::db::Database;
use crate::types::{AuthConfig, BodyType, HistoryItem};
use gpui::{AppContext as _, ClickEvent, Entity, TestAppContext, VisualTestContext};
use gpui_component::Root;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

fn insert(db: &Database, url: &str) -> i64 {
    db.insert_history(
        "GET",
        url,
        r#"[["X-Trace","1"]]"#,
        &BodyType::None,
        &AuthConfig::default(),
    )
    .wait()
    .unwrap()
}

/// Open a window holding a history panel loaded the way startup loads it.
fn panel_window(
    db: Arc<Database>,
    cx: &mut TestAppContext,
) -> (Entity<HistoryPanel>, &mut VisualTestContext) {
    let history = db.load_recent_history(super::HISTORY_LIMIT).wait().unwrap();
    cx.update(gpui_component::init);
    let mut panel = None;
    let (_, cx) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| HistoryPanel::new(db.clone(), history, window, cx));
        panel = Some(view.clone());
        Root::new(view, window, cx)
    });
    (panel.unwrap(), cx)
}

/// Every entry the panel asks the app to open, in order.
fn record_opens(
    panel: &Entity<HistoryPanel>,
    cx: &mut VisualTestContext,
) -> Rc<RefCell<Vec<HistoryItem>>> {
    let opened = Rc::new(RefCell::new(Vec::new()));
    let sink = opened.clone();
    cx.update(|_, cx| {
        cx.subscribe(panel, move |_, event: &HistoryItemClicked, _| {
            sink.borrow_mut().push(event.item.clone())
        })
        .detach();
    });
    opened
}

fn toast_count(cx: &mut VisualTestContext) -> usize {
    cx.update(|window, cx| {
        let root = window.root::<Root>().flatten().unwrap();
        root.read(cx).notification.read(cx).notifications().len()
    })
}

fn click(panel: &Entity<HistoryPanel>, id: i64, cx: &mut VisualTestContext) {
    cx.update(|window, cx| panel.update(cx, |panel, cx| panel.on_item_click(id, window, cx)));
}

#[gpui::test]
fn clicking_a_row_opens_its_full_request(cx: &mut TestAppContext) {
    let db = Arc::new(Database::new_in_memory());
    let id = insert(&db, "https://api.example.test/items");
    let (panel, cx) = panel_window(db.clone(), cx);
    let opened = record_opens(&panel, cx);

    click(&panel, id, cx);
    assert!(
        opened.borrow().is_empty(),
        "the full entry is read off the UI thread"
    );
    crate::test_support::settle(cx, &db);

    let opened = opened.borrow();
    assert_eq!(opened.len(), 1);
    assert_eq!(opened[0].id, id);
    assert_eq!(opened[0].request.url, "https://api.example.test/items");
    assert_eq!(
        opened[0].request.headers,
        vec![("X-Trace".to_string(), "1".to_string())]
    );
}

#[gpui::test]
fn only_the_latest_click_opens(cx: &mut TestAppContext) {
    let db = Arc::new(Database::new_in_memory());
    let first = insert(&db, "https://api.example.test/first");
    let second = insert(&db, "https://api.example.test/second");
    let (panel, cx) = panel_window(db.clone(), cx);
    let opened = record_opens(&panel, cx);

    // The second click lands before the first entry has been read.
    click(&panel, first, cx);
    click(&panel, second, cx);
    crate::test_support::settle(cx, &db);

    let ids: Vec<i64> = opened.borrow().iter().map(|item| item.id).collect();
    assert_eq!(ids, vec![second]);
    cx.update(|_, cx| assert_eq!(panel.read(cx).selected_id, Some(second)));
}

#[gpui::test]
fn clicking_a_deleted_row_reports_it_and_drops_the_row(cx: &mut TestAppContext) {
    let db = Arc::new(Database::new_in_memory());
    let id = insert(&db, "https://api.example.test/gone");
    let (panel, cx) = panel_window(db.clone(), cx);
    let opened = record_opens(&panel, cx);
    // Deleted behind the panel's back, after the list was loaded.
    db.clear_all_history().wait().unwrap();

    click(&panel, id, cx);
    crate::test_support::settle(cx, &db);

    assert!(opened.borrow().is_empty());
    assert_eq!(toast_count(cx), 1);
    cx.update(|_, cx| {
        let panel = panel.read(cx);
        assert!(panel.history.is_empty(), "the stale row is reloaded away");
        assert_eq!(panel.selected_id, None);
    });
}

#[gpui::test]
fn clearing_history_cancels_a_pending_open(cx: &mut TestAppContext) {
    let db = Arc::new(Database::new_in_memory());
    let id = insert(&db, "https://api.example.test/items");
    let (panel, cx) = panel_window(db.clone(), cx);
    let opened = record_opens(&panel, cx);

    cx.update(|window, cx| {
        panel.update(cx, |panel, cx| {
            panel.on_item_click(id, window, cx);
            panel.clear_history(&ClickEvent::default(), window, cx);
        })
    });
    crate::test_support::settle(cx, &db);

    assert!(opened.borrow().is_empty());
    assert_eq!(toast_count(cx), 0);
}

#[gpui::test]
fn failed_clear_restores_rows_and_shows_a_toast(cx: &mut TestAppContext) {
    let db = Arc::new(Database::new_in_memory());
    insert(&db, "https://api.example.test/items");
    // Reads keep working; only the clear's DELETE is rejected.
    db.execute_batch_for_test(
        "CREATE TRIGGER reject_history_delete BEFORE DELETE ON history
         BEGIN SELECT RAISE(ABORT, 'history is read-only'); END;",
    );
    let (panel, cx) = panel_window(db.clone(), cx);

    cx.update(|window, cx| {
        panel.update(cx, |panel, cx| {
            panel.clear_history(&ClickEvent::default(), window, cx);
            assert!(panel.history.is_empty(), "the clear is optimistic");
        })
    });
    crate::test_support::settle(cx, &db);

    cx.update(|_, cx| assert_eq!(panel.read(cx).history.len(), 1));
    assert_eq!(toast_count(cx), 1);
}
