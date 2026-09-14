use super::{CollectionsPanel, NodeRef, SavedRequestClicked};
use crate::db::Database;
use crate::types::{HttpMethod, RequestData};
use gpui::{AppContext as _, TestAppContext};
use std::{cell::RefCell, rc::Rc, sync::Arc};

#[gpui::test]
fn opening_by_id_uses_current_nested_request_and_ignores_deleted_requests(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let db = Arc::new(Database::new_in_memory());
    let collection = db.create_collection("Collection").unwrap();
    let folder = db.create_folder(collection, None, "Folder").unwrap();
    let request = RequestData::new(HttpMethod::GET, "https://example.test/original".into());
    let id = db
        .insert_saved_request(collection, Some(folder), "Request", &request, &[], &[])
        .unwrap();
    let collections = db.load_collections().unwrap();
    let cx = cx.add_empty_window();
    let panel = cx.update(|window, cx| {
        cx.new(|cx| CollectionsPanel::new(db.clone(), collections, window, cx))
    });
    let opened = Rc::new(RefCell::new(Vec::new()));
    let _subscription = cx.update(|_, cx| {
        let opened = opened.clone();
        cx.subscribe(&panel, move |_, event: &SavedRequestClicked, _| {
            opened.borrow_mut().push(event.request.clone())
        })
    });

    let updated = RequestData::new(HttpMethod::POST, "https://example.test/updated".into());
    db.update_saved_request(id, collection, Some(folder), "Renamed", &updated, &[], &[])
        .unwrap();
    let reloaded = db.load_collections().unwrap();
    cx.update(|_, cx| {
        panel.update(cx, |panel, cx| {
            panel.apply_loaded_collections(reloaded, cx);
            panel.open_request(id, cx);
        })
    });
    assert_eq!(opened.borrow().len(), 1);
    assert_eq!(opened.borrow()[0].id, id);
    assert_eq!(opened.borrow()[0].name, "Renamed");
    assert_eq!(opened.borrow()[0].request, updated);
    cx.update(|_, cx| assert_eq!(panel.read(cx).selected, Some(NodeRef::Request(id))));

    cx.update(|_, cx| {
        panel.update(cx, |panel, cx| {
            panel.apply_loaded_collections(Vec::new(), cx);
            panel.open_request(id, cx);
            assert_eq!(panel.selected, None);
        })
    });
    assert_eq!(opened.borrow().len(), 1);
}
