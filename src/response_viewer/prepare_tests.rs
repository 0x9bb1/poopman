//! Scheduling of background body prepares: deduplicated by response identity,
//! bounded in number, and only the visible response may wait for a slot.

use super::{MAX_BODY_PREPARES, ResponseViewer};
use crate::types::ResponseData;
use gpui::{AppContext as _, Entity, TestAppContext, VisualTestContext};
use gpui_component::Root;
use std::sync::Arc;

fn json(body: &str) -> Arc<ResponseData> {
    Arc::new(ResponseData {
        status: Some(200),
        duration_ms: 1,
        headers: Vec::new(),
        body: body.as_bytes().to_vec(),
        is_text: true,
        downloaded_to: None,
        downloaded_bytes: None,
    })
}

fn viewer_window(cx: &mut TestAppContext) -> (Entity<ResponseViewer>, &mut VisualTestContext) {
    cx.update(gpui_component::init);
    let mut viewer = None;
    let (_, cx) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| ResponseViewer::new(window, cx));
        viewer = Some(view.clone());
        Root::new(view, window, cx)
    });
    (viewer.unwrap(), cx)
}

/// Show each response in turn within one update, i.e. faster than any
/// background prepare can finish.
fn show_quickly(
    viewer: &Entity<ResponseViewer>,
    responses: &[&Arc<ResponseData>],
    cx: &mut VisualTestContext,
) {
    cx.update(|window, cx| {
        viewer.update(cx, |viewer, cx| {
            for response in responses {
                viewer.set_response((*response).clone(), window, cx);
            }
        })
    });
}

fn is_running(viewer: &ResponseViewer, response: &Arc<ResponseData>) -> bool {
    viewer
        .body_prepares
        .iter()
        .any(|running| Arc::ptr_eq(running, response))
}

fn is_cached(viewer: &ResponseViewer, response: &Arc<ResponseData>) -> bool {
    viewer
        .body_cache
        .iter()
        .any(|cached| Arc::ptr_eq(&cached.response, response))
}

#[gpui::test]
fn showing_a_response_again_joins_its_running_prepare(cx: &mut TestAppContext) {
    let (viewer, cx) = viewer_window(cx);
    let a = json(r#"{"a":1}"#);
    let b = json(r#"{"b":2}"#);

    show_quickly(&viewer, &[&a, &b, &a], cx);
    cx.update(|_, cx| {
        let viewer = viewer.read(cx);
        assert_eq!(viewer.body_prepares.len(), 2, "A is prepared once");
        assert!(viewer.body_loading);
    });
    cx.run_until_parked();

    cx.update(|_, cx| {
        let viewer = viewer.read(cx);
        assert!(viewer.body_prepares.is_empty());
        assert!(viewer.body_ready);
        assert_eq!(viewer.body_display.read(cx).value(), "{\n    \"a\": 1\n}");
    });
}

#[gpui::test]
fn only_the_visible_response_waits_for_a_prepare_slot(cx: &mut TestAppContext) {
    let (viewer, cx) = viewer_window(cx);
    assert_eq!(MAX_BODY_PREPARES, 2, "the scenario below fills both slots");
    let a = json(r#"{"a":1}"#);
    let b = json(r#"{"b":2}"#);
    let c = json(r#"{"c":3}"#);
    let d = json(r#"{"d":4}"#);

    show_quickly(&viewer, &[&a, &b, &c, &d], cx);
    cx.update(|_, cx| {
        let viewer = viewer.read(cx);
        assert!(is_running(viewer, &a) && is_running(viewer, &b));
        assert_eq!(viewer.body_prepares.len(), 2);
        let waiting = viewer.body_prepare_waiting.as_ref().unwrap();
        assert!(Arc::ptr_eq(waiting, &d), "C was replaced before it started");
    });
    cx.run_until_parked();

    cx.update(|_, cx| {
        let viewer = viewer.read(cx);
        assert_eq!(viewer.body_display.read(cx).value(), "{\n    \"d\": 4\n}");
        assert!(viewer.body_prepare_waiting.is_none());
        // A and B finished in the background and are cached for a revisit;
        // C was never prepared.
        assert!(is_cached(viewer, &a) && is_cached(viewer, &b) && is_cached(viewer, &d));
        assert!(!is_cached(viewer, &c));
    });
}

#[gpui::test]
fn clearing_the_viewer_drops_the_waiting_prepare(cx: &mut TestAppContext) {
    let (viewer, cx) = viewer_window(cx);
    let a = json(r#"{"a":1}"#);
    let b = json(r#"{"b":2}"#);
    let c = json(r#"{"c":3}"#);

    show_quickly(&viewer, &[&a, &b, &c], cx);
    cx.update(|window, cx| viewer.update(cx, |viewer, cx| viewer.clear_response(window, cx)));
    cx.run_until_parked();

    cx.update(|_, cx| {
        let viewer = viewer.read(cx);
        assert!(viewer.body_prepares.is_empty());
        assert!(viewer.body_prepare_waiting.is_none());
        assert!(!viewer.body_ready);
        assert!(!is_cached(viewer, &c));
    });
}
