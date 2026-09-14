use super::RequestEditor;
use crate::test_support::SubscriptionTracker;
use crate::types::{
    AppSettings, BodyType, HeaderState, HeaderType, HttpMethod, ParamState, PredefinedHeader,
    RawSubtype, RequestData,
};
use gpui::{AppContext as _, ClickEvent, TestAppContext};
use gpui_component::input::InputEvent;
use std::{
    cell::Cell,
    rc::Rc,
    sync::{Arc, RwLock},
};

fn header() -> HeaderState {
    HeaderState {
        enabled: true,
        key: "X-Test".into(),
        value: "value".into(),
        header_type: HeaderType::Custom,
        predefined: None,
    }
}

fn param() -> ParamState {
    ParamState {
        enabled: true,
        key: "q".into(),
        value: "value".into(),
    }
}

#[gpui::test]
fn row_removal_and_repeated_request_loads_release_listeners(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let cx = cx.add_empty_window();
    let editor = cx.update(|window, cx| {
        cx.new(|cx| RequestEditor::new(Arc::new(RwLock::new(AppSettings::default())), window, cx))
    });
    let tracker = SubscriptionTracker::default();
    let request = RequestData::new(HttpMethod::POST, "https://example.test/?q=loaded".into());
    for _ in 0..20 {
        cx.update(|window, cx| {
            editor.update(cx, |editor, cx| {
                editor.load_headers_state(&[header()], window, cx);
                editor.load_params_state(&[param()], window, cx);
                for row in &mut editor.headers {
                    tracker.track(&mut row._subscriptions);
                }
                for row in &mut editor.params {
                    tracker.track(&mut row._subscriptions);
                }
                assert_eq!(tracker.live(), 6);
                let index = editor
                    .headers
                    .iter()
                    .position(|row| row.header_type == HeaderType::Custom)
                    .unwrap();
                editor.remove_header_row(index, &ClickEvent::default(), window, cx);
                editor.remove_param(0, window, cx);
                assert_eq!(tracker.live(), 3);
                editor.load_request(&request, window, cx);
                assert_eq!(tracker.live(), 0);
            })
        });
    }
    // Permanent body and URL listeners must survive all the row replacements.
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.body_editor.update(cx, |body, cx| {
                body.set_body(
                    &BodyType::Raw {
                        content: "plain text".into(),
                        subtype: RawSubtype::Text,
                    },
                    window,
                    cx,
                )
            });
        })
    });
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        let content_type = editor
            .headers
            .iter()
            .find(|row| row.predefined == Some(PredefinedHeader::ContentType))
            .unwrap();
        assert_eq!(
            content_type.value_input.read(cx).value().as_str(),
            "text/plain"
        );
        assert_eq!(
            editor.params[0].value_input.read(cx).value().as_str(),
            "loaded"
        );
    });
}

#[gpui::test]
fn focused_url_edits_release_every_replaced_param_subscription(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let cx = cx.add_empty_window();
    let editor = cx.update(|window, cx| {
        cx.new(|cx| RequestEditor::new(Arc::new(RwLock::new(AppSettings::default())), window, cx))
    });
    let tracker = SubscriptionTracker::default();
    for index in 0..20 {
        cx.update(|window, cx| {
            editor.update(cx, |editor, cx| {
                for row in &mut editor.params {
                    tracker.track(&mut row._subscriptions);
                }
                assert!(tracker.live() > 0);
                editor.url_input.update(cx, |input, cx| {
                    input.focus(window, cx);
                    input.set_value(format!("https://example.test/?q={index}"), window, cx);
                });
            })
        });
        assert_eq!(tracker.live(), 0);
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            assert_eq!(editor.params.len(), 2);
            assert_eq!(
                editor.params[0].value_input.read(cx).value().as_str(),
                index.to_string()
            );
        });
    }
}

#[gpui::test]
fn removed_param_events_do_not_sync_url_but_live_rows_still_do(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let cx = cx.add_empty_window();
    let editor = cx.update(|window, cx| {
        cx.new(|cx| RequestEditor::new(Arc::new(RwLock::new(AppSettings::default())), window, cx))
    });
    let (removed, live, url) = cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.load_request(
                &RequestData::new(HttpMethod::GET, "https://example.test/".into()),
                window,
                cx,
            );
            editor.load_params_state(&[param()], window, cx);
            let removed = editor.params[0].key_input.clone();
            editor.remove_param(0, window, cx);
            let live = editor.params.last().unwrap().key_input.clone();
            live.update(cx, |input, cx| input.focus(window, cx));
            (removed, live, editor.url_input.clone())
        })
    });
    let changes = Rc::new(Cell::new(0));
    let _subscription = cx.update(|_, cx| {
        let changes = changes.clone();
        cx.subscribe(&url, move |_, event: &InputEvent, _| {
            if matches!(event, InputEvent::Change) {
                changes.set(changes.get() + 1);
            }
        })
    });
    // Keep the old entity alive to prove the listener itself was cancelled.
    cx.update(|_, cx| removed.update(cx, |_, cx| cx.emit(InputEvent::Change)));
    assert_eq!(changes.get(), 0);
    cx.update(|window, cx| live.update(cx, |input, cx| input.set_value("active", window, cx)));
    assert!(changes.get() > 0);
    cx.update(|_, cx| {
        assert!(url.read(cx).value().contains("active="));
        assert!(
            editor
                .read(cx)
                .params
                .last()
                .unwrap()
                .key_input
                .read(cx)
                .value()
                .is_empty()
        );
    });
}
