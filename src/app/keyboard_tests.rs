//! Keyboard access to the custom controls: tabs, section pills and the
//! body/auth radios. Mouse clicks are covered too, because `track_focus`
//! changes what a click does to focus.

use super::PoopmanApp;
use super::save_tests::setup;
use super::tab_state_tests::{response, select_sections};
use crate::db::Database;
use crate::types::{
    AuthType, BodyType, HttpMethod, ParamState, RequestData, RequestSection, ResponseSection,
};
use gpui::{Entity, FocusHandle, KeyUpEvent, Keystroke, Modifiers, VisualTestContext};
use std::sync::Arc;

fn focus(handle: &FocusHandle, cx: &mut VisualTestContext) {
    cx.update(|window, _| handle.focus(window));
    cx.run_until_parked();
}

fn is_focused(handle: &FocusHandle, cx: &mut VisualTestContext) -> bool {
    cx.update(|window, _| handle.is_focused(window))
}

/// A full key press. `simulate_keystrokes` only sends the key-down, and gpui
/// activates a focused control on key-up.
fn press(key: &str, cx: &mut VisualTestContext) {
    cx.simulate_keystrokes(key);
    cx.simulate_event(KeyUpEvent {
        keystroke: Keystroke::parse(key).unwrap(),
    });
    cx.run_until_parked();
}

fn click(selector: &'static str, cx: &mut VisualTestContext) {
    let position = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is not rendered"))
        .center();
    cx.simulate_mouse_move(position, None, Modifiers::none());
    cx.simulate_click(position, Modifiers::none());
    cx.run_until_parked();
}

fn request_section(app: &Entity<PoopmanApp>, cx: &mut VisualTestContext) -> RequestSection {
    cx.update(|_, cx| {
        app.read(cx)
            .request_editor
            .read(cx)
            .capture_state(cx)
            .section
    })
}

fn response_section(app: &Entity<PoopmanApp>, cx: &mut VisualTestContext) -> ResponseSection {
    cx.update(|_, cx| {
        app.read(cx)
            .response_viewer
            .read(cx)
            .capture_state()
            .section
    })
}

fn request(app: &Entity<PoopmanApp>, cx: &mut VisualTestContext) -> RequestData {
    cx.update(|_, cx| {
        app.read(cx)
            .request_editor
            .read(cx)
            .get_current_request_data(cx)
    })
}

fn tab_count(app: &Entity<PoopmanApp>, cx: &mut VisualTestContext) -> usize {
    cx.update(|_, cx| app.read(cx).request_tabs.len())
}

fn active_tab(app: &Entity<PoopmanApp>, cx: &mut VisualTestContext) -> usize {
    cx.update(|_, cx| app.read(cx).active_tab_index)
}

fn request_pill(
    app: &Entity<PoopmanApp>,
    section: RequestSection,
    cx: &mut VisualTestContext,
) -> FocusHandle {
    cx.update(|_, cx| app.read(cx).request_editor.read(cx).section_focus(section))
}

fn response_pill(
    app: &Entity<PoopmanApp>,
    section: ResponseSection,
    cx: &mut VisualTestContext,
) -> FocusHandle {
    cx.update(|_, cx| app.read(cx).response_viewer.read(cx).section_focus(section))
}

fn tab_handle(app: &Entity<PoopmanApp>, index: usize, cx: &mut VisualTestContext) -> FocusHandle {
    cx.update(|_, cx| app.read(cx).tab_bar.read(cx).tab_focus(index))
}

fn close_handle(app: &Entity<PoopmanApp>, index: usize, cx: &mut VisualTestContext) -> FocusHandle {
    cx.update(|_, cx| app.read(cx).tab_bar.read(cx).close_focus(index))
}

fn new_tab_handle(app: &Entity<PoopmanApp>, cx: &mut VisualTestContext) -> FocusHandle {
    cx.update(|_, cx| app.read(cx).tab_bar.read(cx).new_tab_focus())
}

#[gpui::test]
fn request_section_pills_move_with_arrows_and_select_with_enter_or_space(
    cx: &mut gpui::TestAppContext,
) {
    let (app, cx) = setup(cx, Arc::new(Database::new_in_memory()));
    cx.run_until_parked();
    let headers = request_pill(&app, RequestSection::Headers, cx);
    let auth = request_pill(&app, RequestSection::Auth, cx);
    let body = request_pill(&app, RequestSection::Body, cx);
    focus(&headers, cx);

    // Arrows move focus only; the section changes on activation.
    press("right", cx);
    assert!(is_focused(&auth, cx));
    assert_eq!(request_section(&app, cx), RequestSection::Headers);
    press("enter", cx);
    assert_eq!(request_section(&app, cx), RequestSection::Auth);

    press("right", cx);
    press("right", cx);
    assert!(is_focused(&body, cx));
    press("space", cx);
    assert_eq!(request_section(&app, cx), RequestSection::Body);

    // Wraps at both ends.
    press("right", cx);
    assert!(is_focused(&headers, cx));
    press("left", cx);
    assert!(is_focused(&body, cx));
}

#[gpui::test]
fn response_section_pills_move_with_arrows_and_select_with_enter(cx: &mut gpui::TestAppContext) {
    let (app, cx) = setup(cx, Arc::new(Database::new_in_memory()));
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.response_viewer.update(cx, |viewer, cx| {
                viewer.set_response(response(200), window, cx);
            });
        })
    });
    cx.run_until_parked();
    let body = response_pill(&app, ResponseSection::Body, cx);
    let headers = response_pill(&app, ResponseSection::Headers, cx);
    focus(&body, cx);

    press("right", cx);
    assert!(is_focused(&headers, cx));
    assert_eq!(response_section(&app, cx), ResponseSection::Body);
    press("enter", cx);
    assert_eq!(response_section(&app, cx), ResponseSection::Headers);
}

#[gpui::test]
fn body_type_radios_follow_the_arrow_keys(cx: &mut gpui::TestAppContext) {
    let (app, cx) = setup(cx, Arc::new(Database::new_in_memory()));
    select_sections(&app, cx, RequestSection::Body, ResponseSection::Body);
    let radio = |index: usize, cx: &mut VisualTestContext| {
        cx.update(|_, cx| {
            let editor = app.read(cx).request_editor.read(cx);
            editor.body_editor().read(cx).body_type_focus(index)
        })
    };
    // "raw" is the default selection.
    let raw = radio(1, cx);
    focus(&raw, cx);

    press("right", cx);
    assert!(is_focused(&radio(2, cx), cx));
    assert!(matches!(request(&app, cx).body, BodyType::FormData(_)));

    press("left", cx);
    press("left", cx);
    assert!(is_focused(&radio(0, cx), cx));
    assert!(matches!(request(&app, cx).body, BodyType::None));
}

#[gpui::test]
fn auth_type_radios_follow_the_arrow_keys(cx: &mut gpui::TestAppContext) {
    let (app, cx) = setup(cx, Arc::new(Database::new_in_memory()));
    select_sections(&app, cx, RequestSection::Auth, ResponseSection::Body);
    let radio = |index: usize, cx: &mut VisualTestContext| {
        cx.update(|_, cx| {
            let editor = app.read(cx).request_editor.read(cx);
            editor.auth_editor().read(cx).auth_type_focus(index)
        })
    };
    focus(&radio(0, cx), cx);

    press("right", cx);
    assert!(is_focused(&radio(1, cx), cx));
    assert_eq!(request(&app, cx).auth.auth_type, AuthType::Bearer);

    press("left", cx);
    press("left", cx);
    assert!(is_focused(&radio(3, cx), cx));
    assert_eq!(request(&app, cx).auth.auth_type, AuthType::ApiKey);
}

#[gpui::test]
fn request_tabs_move_with_arrows_and_select_with_enter(cx: &mut gpui::TestAppContext) {
    let (app, cx) = setup(cx, Arc::new(Database::new_in_memory()));
    cx.simulate_keystrokes("secondary-t secondary-t");
    cx.run_until_parked();
    assert_eq!(active_tab(&app, cx), 2);
    focus(&tab_handle(&app, 2, cx), cx);

    press("left", cx);
    assert!(is_focused(&tab_handle(&app, 1, cx), cx));
    assert_eq!(active_tab(&app, cx), 2, "arrows only move focus");
    press("enter", cx);
    assert_eq!(active_tab(&app, cx), 1);

    press("right", cx);
    press("right", cx);
    assert!(
        is_focused(&tab_handle(&app, 0, cx), cx),
        "wraps to the first"
    );
    press("space", cx);
    assert_eq!(active_tab(&app, cx), 0);
}

#[gpui::test]
fn new_tab_button_activates_from_the_keyboard(cx: &mut gpui::TestAppContext) {
    let (app, cx) = setup(cx, Arc::new(Database::new_in_memory()));
    cx.run_until_parked();
    focus(&new_tab_handle(&app, cx), cx);

    press("enter", cx);
    assert_eq!(tab_count(&app, cx), 2);
    press("space", cx);
    assert_eq!(tab_count(&app, cx), 3);
}

/// Closing a tab from its focused close button removes the focused element.
/// Shortcuts must keep working afterwards.
#[gpui::test]
fn closing_a_tab_from_its_focused_close_button_keeps_shortcuts_alive(
    cx: &mut gpui::TestAppContext,
) {
    let (app, cx) = setup(cx, Arc::new(Database::new_in_memory()));
    cx.simulate_keystrokes("secondary-t");
    cx.run_until_parked();
    assert_eq!(tab_count(&app, cx), 2);
    focus(&close_handle(&app, 1, cx), cx);

    press("enter", cx);
    assert_eq!(tab_count(&app, cx), 1);

    cx.simulate_keystrokes("secondary-t");
    assert_eq!(tab_count(&app, cx), 2);
}

/// Presses Tab up to `presses` times and returns the names of the controls
/// that never received focus.
fn unreached_by_tab(
    mut wanted: Vec<(&'static str, FocusHandle)>,
    presses: usize,
    cx: &mut VisualTestContext,
) -> Vec<&'static str> {
    for _ in 0..presses {
        cx.simulate_keystrokes("tab");
        cx.run_until_parked();
        wanted.retain(|(_, handle)| !cx.update(|window, _| handle.is_focused(window)));
    }
    wanted.into_iter().map(|(name, _)| name).collect()
}

/// From the app root, Tab alone must reach one control of every group. The
/// groups rove internally, so only their selected member is a stop.
#[gpui::test]
fn tab_key_reaches_every_control_group(cx: &mut gpui::TestAppContext) {
    let (app, cx) = setup(cx, Arc::new(Database::new_in_memory()));
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.response_viewer.update(cx, |viewer, cx| {
                viewer.set_response(response(200), window, cx);
            });
        })
    });
    select_sections(&app, cx, RequestSection::Headers, ResponseSection::Headers);

    let wanted = vec![
        ("active request tab", tab_handle(&app, 0, cx)),
        ("its close button", close_handle(&app, 0, cx)),
        ("new tab button", new_tab_handle(&app, cx)),
        (
            "active request section",
            request_pill(&app, RequestSection::Headers, cx),
        ),
        (
            "active response section",
            response_pill(&app, ResponseSection::Headers, cx),
        ),
    ];
    assert_eq!(unreached_by_tab(wanted, 80, cx), Vec::<&str>::new());

    // Roving: the unselected members of a group are not stops.
    let unselected = vec![
        (
            "unselected request section",
            request_pill(&app, RequestSection::Body, cx),
        ),
        (
            "unselected response section",
            response_pill(&app, ResponseSection::Body, cx),
        ),
    ];
    assert_eq!(unreached_by_tab(unselected, 80, cx).len(), 2);
}

/// The radio groups sit before their section's editor in the Tab order.
#[gpui::test]
fn tab_key_reaches_the_selected_body_and_auth_radio(cx: &mut gpui::TestAppContext) {
    let (app, cx) = setup(cx, Arc::new(Database::new_in_memory()));

    select_sections(&app, cx, RequestSection::Body, ResponseSection::Body);
    let body_radio = cx.update(|_, cx| {
        let editor = app.read(cx).request_editor.read(cx);
        editor.body_editor().read(cx).body_type_focus(1)
    });
    assert_eq!(
        unreached_by_tab(vec![("selected body type", body_radio)], 80, cx),
        Vec::<&str>::new()
    );

    select_sections(&app, cx, RequestSection::Auth, ResponseSection::Body);
    let auth_radio = cx.update(|_, cx| {
        let editor = app.read(cx).request_editor.read(cx);
        editor.auth_editor().read(cx).auth_type_focus(0)
    });
    assert_eq!(
        unreached_by_tab(vec![("selected auth type", auth_radio)], 80, cx),
        Vec::<&str>::new()
    );
}

/// `track_focus` makes gpui focus an element on mouse-down. The controls
/// suppress that, so clicking one must not pull focus out of an input.
#[gpui::test]
fn clicking_a_section_pill_selects_it_without_taking_focus(cx: &mut gpui::TestAppContext) {
    let (app, cx) = setup(cx, Arc::new(Database::new_in_memory()));
    cx.simulate_keystrokes("secondary-l");
    cx.run_until_parked();

    click("tab-body", cx);
    assert_eq!(request_section(&app, cx), RequestSection::Body);
    assert!(!is_focused(
        &request_pill(&app, RequestSection::Body, cx),
        cx
    ));

    // Still in the URL field: typing lands there.
    cx.simulate_input("https://typed.test");
    cx.run_until_parked();
    assert_eq!(request(&app, cx).url, "https://typed.test");
}

/// URL↔Params sync is arbitrated by which input has focus, and a click on a
/// tab no longer moves focus. Switching tabs by mouse with the URL focused
/// must still restore each tab's own rows, disabled ones included.
#[gpui::test]
fn clicking_tabs_with_the_url_focused_keeps_each_tabs_params(cx: &mut gpui::TestAppContext) {
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

    cx.simulate_keystrokes("secondary-t secondary-l");
    cx.run_until_parked();
    assert_eq!(active_tab(&app, cx), 1);

    click("request-tab-0", cx);
    assert_eq!(active_tab(&app, cx), 0);
    assert_eq!(request(&app, cx).url, "https://example.test/a?x=1");
    assert_eq!(params(cx), tab_a_params);

    click("request-tab-1", cx);
    assert_eq!(active_tab(&app, cx), 1);
    click("request-tab-0", cx);
    assert_eq!(params(cx), tab_a_params);
}
