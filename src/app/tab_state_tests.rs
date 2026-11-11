use super::PoopmanApp;
use super::save_tests::{edited_request, setup};
use crate::db::Database;
use crate::request_editor::{RequestCompleted, RequestStarted};
use crate::request_tab::ResponseTabState;
use crate::types::{
    HttpMethod, ParamState, RequestData, RequestSection, ResponseData, ResponseSection,
};
use gpui::{Entity, VisualTestContext};
use std::sync::Arc;

pub(super) fn response(status: u16) -> Arc<ResponseData> {
    Arc::new(ResponseData {
        status: Some(status),
        duration_ms: 1,
        headers: vec![("X-Status".into(), status.to_string())],
        body: b"{}".to_vec(),
        is_text: true,
        downloaded_to: None,
        downloaded_bytes: None,
    })
}

/// What the user would see for the active tab: editor contents, both
/// sections, and the response panel.
fn shown(
    app: &Entity<PoopmanApp>,
    cx: &mut VisualTestContext,
) -> (
    RequestData,
    Vec<ParamState>,
    RequestSection,
    ResponseTabState,
) {
    cx.update(|_, cx| {
        let app = app.read(cx);
        let editor = app.request_editor.read(cx).capture_state(cx);
        let viewer = app.response_viewer.read(cx).capture_state();
        (
            editor.request,
            editor.params_state.unwrap_or_default(),
            editor.section,
            viewer,
        )
    })
}

/// Stand in for the user clicking a section pill in each panel.
pub(super) fn select_sections(
    app: &Entity<PoopmanApp>,
    cx: &mut VisualTestContext,
    request: RequestSection,
    response: ResponseSection,
) {
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.request_editor.update(cx, |editor, cx| {
                let mut state = editor.capture_state(cx);
                state.section = request;
                editor.restore_state(&state, window, cx);
            });
            app.response_viewer.update(cx, |viewer, cx| {
                let mut state = viewer.capture_state();
                state.section = response;
                viewer.restore_state(&state, window, cx);
            });
        })
    });
    cx.run_until_parked();
}

#[gpui::test]
fn switching_tabs_restores_each_tabs_own_state_without_leaking(cx: &mut gpui::TestAppContext) {
    let (app, cx) = setup(cx, Arc::new(Database::new_in_memory()));
    let disabled_param = ParamState {
        enabled: false,
        key: "draft".into(),
        value: "kept".into(),
    };

    // Tab A: an edited request with a disabled param, a response, and both
    // panels moved off their default sections.
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.request_editor.update(cx, |editor, cx| {
                editor.load_request(&edited_request(), window, cx);
                let mut params = editor.get_params_state(cx);
                params.push(disabled_param.clone());
                editor.load_params_state(&params, window, cx);
            });
            app.response_viewer.update(cx, |viewer, cx| {
                viewer.set_response(response(201), window, cx);
            });
        })
    });
    cx.run_until_parked();
    select_sections(&app, cx, RequestSection::Body, ResponseSection::Headers);
    let tab_a = shown(&app, cx);
    assert_eq!(tab_a.2, RequestSection::Body);
    assert_eq!(tab_a.3.section, ResponseSection::Headers);
    assert!(tab_a.1.contains(&disabled_param));

    // Tab B starts from defaults, not from what tab A left in the shared panels.
    cx.update(|window, cx| app.update(cx, |app, cx| app.create_new_tab(window, cx)));
    cx.run_until_parked();
    let tab_b = shown(&app, cx);
    assert_eq!(tab_b.0.url, "");
    assert!(tab_b.1.is_empty());
    assert_eq!(tab_b.2, RequestSection::default());
    assert!(tab_b.3.response.is_none());
    assert_eq!(tab_b.3.section, ResponseSection::default());
    select_sections(&app, cx, RequestSection::Auth, ResponseSection::Body);

    // Back to A: everything it had, exactly.
    cx.update(|window, cx| app.update(cx, |app, cx| app.switch_to_tab(0, window, cx)));
    cx.run_until_parked();
    let restored_a = shown(&app, cx);
    assert_eq!(restored_a.0, tab_a.0);
    assert_eq!(restored_a.1, tab_a.1);
    assert_eq!(restored_a.2, RequestSection::Body);
    assert_eq!(restored_a.3.section, ResponseSection::Headers);
    assert_eq!(
        restored_a.3.response.map(|r| r.status),
        Some(Some(201)),
        "tab A keeps its own response"
    );

    // And B kept its own section, with nothing of A's.
    cx.update(|window, cx| app.update(cx, |app, cx| app.switch_to_tab(1, window, cx)));
    cx.run_until_parked();
    let restored_b = shown(&app, cx);
    assert_eq!(restored_b.0.url, "");
    assert_eq!(restored_b.2, RequestSection::Auth);
    assert!(restored_b.3.response.is_none());
    cx.update(|_, cx| {
        assert!(
            !app.read(cx).request_tabs[1].is_dirty(),
            "section changes are UI state and must not dirty a tab"
        );
    });
}

#[gpui::test]
fn a_response_arriving_in_the_background_reopens_its_body(cx: &mut gpui::TestAppContext) {
    let (app, cx) = setup(cx, Arc::new(Database::new_in_memory()));
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.request_editor.update(cx, |editor, cx| {
                editor.load_request(&edited_request(), window, cx);
            });
            app.response_viewer.update(cx, |viewer, cx| {
                viewer.set_response(response(200), window, cx);
            });
        })
    });
    cx.run_until_parked();
    select_sections(&app, cx, RequestSection::Params, ResponseSection::Headers);
    let tab_a = cx.update(|_, cx| app.read(cx).request_tabs[0].id);

    // A request from tab A is in flight while the user looks at tab B.
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.request_editor.update(cx, |_, cx| {
                cx.emit(RequestStarted {
                    tab_id: tab_a,
                    request_id: 7,
                    request: edited_request(),
                })
            })
        })
    });
    cx.run_until_parked();
    cx.update(|window, cx| app.update(cx, |app, cx| app.create_new_tab(window, cx)));
    cx.run_until_parked();
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.request_editor.update(cx, |_, cx| {
                cx.emit(RequestCompleted {
                    tab_id: tab_a,
                    request_id: 7,
                    history_request: edited_request(),
                    response: response(404),
                })
            })
        })
    });
    cx.run_until_parked();
    assert!(
        shown(&app, cx).3.response.is_none(),
        "tab B's panel is untouched"
    );

    cx.update(|window, cx| app.update(cx, |app, cx| app.switch_to_tab(0, window, cx)));
    cx.run_until_parked();
    let restored = shown(&app, cx);
    assert_eq!(restored.2, RequestSection::Params, "request section kept");
    assert_eq!(restored.3.section, ResponseSection::Body);
    assert_eq!(restored.3.response.map(|r| r.status), Some(Some(404)));
}

/// URL↔Params sync is arbitrated by which input has focus, on the assumption
/// that a programmatic load never happens while the URL input holds it. A
/// tab-switching shortcut pressed while typing in the URL breaks that
/// assumption; the restored tab must still keep its disabled params.
#[gpui::test]
fn switching_tabs_by_shortcut_from_the_url_field_keeps_disabled_params(
    cx: &mut gpui::TestAppContext,
) {
    let (app, cx) = setup(cx, Arc::new(Database::new_in_memory()));
    let disabled = ParamState {
        enabled: false,
        key: "draft".into(),
        value: "kept".into(),
    };
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.request_editor.update(cx, |editor, cx| {
                let request =
                    RequestData::new(HttpMethod::GET, "https://example.test/a?x=1".into());
                editor.load_request(&request, window, cx);
                let mut params = editor.get_params_state(cx);
                params.push(disabled.clone());
                editor.load_params_state(&params, window, cx);
            })
        })
    });
    cx.run_until_parked();
    let params = |cx: &mut VisualTestContext| {
        cx.update(|_, cx| app.read(cx).request_editor.read(cx).get_params_state(cx))
    };
    let tab_a_params = params(cx);
    assert!(tab_a_params.contains(&disabled));

    // New tab, focus its URL field, then go back to the first tab.
    cx.simulate_keystrokes("secondary-t secondary-l");
    cx.run_until_parked();
    cx.simulate_keystrokes("ctrl-shift-tab");
    cx.run_until_parked();

    assert_eq!(cx.update(|_, cx| app.read(cx).active_tab_index), 0);
    assert_eq!(params(cx), tab_a_params);
}
