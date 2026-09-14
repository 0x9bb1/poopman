use super::BodyEditor;
use crate::test_support::SubscriptionTracker;
use crate::types::{BodyType, FormDataRow, FormDataValue};
use gpui::{AppContext as _, TestAppContext};
use gpui_component::{input::InputEvent, select::SelectEvent};

fn populated_row() -> FormDataRow {
    FormDataRow {
        enabled: true,
        key: "name".into(),
        value: FormDataValue::Text("value".into()),
    }
}

#[gpui::test]
fn deleting_and_reloading_form_rows_releases_listeners(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let cx = cx.add_empty_window();
    let editor = cx.update(|window, cx| cx.new(|cx| BodyEditor::new(window, cx)));
    let tracker = SubscriptionTracker::default();
    for _ in 0..20 {
        cx.update(|window, cx| {
            editor.update(cx, |editor, cx| {
                editor.set_body(&BodyType::FormData(vec![populated_row()]), window, cx);
                assert_eq!(editor.formdata_rows.len(), 2);
                for row in &mut editor.formdata_input_states {
                    tracker.track(&mut row._subscriptions);
                }
                assert_eq!(tracker.live(), 6);
                editor.remove_formdata_row(editor.formdata_row_ids[0], window, cx);
                assert_eq!(tracker.live(), 3);
                editor.set_body(&BodyType::FormData(vec![populated_row()]), window, cx);
                assert_eq!(tracker.live(), 0);
            })
        });
    }
}

#[gpui::test]
fn clearing_form_row_truncates_unused_listeners_and_preserves_auto_add(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let cx = cx.add_empty_window();
    let editor = cx.update(|window, cx| cx.new(|cx| BodyEditor::new(window, cx)));
    let tracker = SubscriptionTracker::default();
    let first = cx.update(|_, cx| editor.read(cx).formdata_input_states[0].key_input.clone());
    for _ in 0..20 {
        cx.update(|window, cx| first.update(cx, |input, cx| input.set_value("name", window, cx)));
        cx.update(|_, cx| {
            editor.update(cx, |editor, _| {
                assert_eq!(editor.formdata_rows.len(), 2);
                tracker.track(&mut editor.formdata_input_states[1]._subscriptions);
                assert_eq!(tracker.live(), 3);
            })
        });
        cx.update(|window, cx| first.update(cx, |input, cx| input.set_value("", window, cx)));
        assert_eq!(tracker.live(), 0);
        cx.update(|_, cx| assert_eq!(editor.read(cx).formdata_rows.len(), 1));
    }
}

#[gpui::test]
fn removed_form_controls_cannot_edit_replacement_rows(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let cx = cx.add_empty_window();
    let editor = cx.update(|window, cx| cx.new(|cx| BodyEditor::new(window, cx)));
    let old = cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.set_body(&BodyType::FormData(vec![populated_row()]), window, cx);
            let inputs = &editor.formdata_input_states[0];
            let old = (inputs.key_input.clone(), inputs.type_select.clone());
            editor.remove_formdata_row(editor.formdata_row_ids[0], window, cx);
            editor.set_body(&BodyType::FormData(vec![populated_row()]), window, cx);
            old
        })
    });
    cx.update(|window, cx| {
        old.0.update(cx, |input, cx| {
            input.set_value("stale", window, cx);
            cx.emit(InputEvent::Change);
        });
        old.1
            .update(cx, |_, cx| cx.emit(SelectEvent::Confirm(Some("File"))));
    });
    let live_select = cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert_eq!(editor.get_draft(cx).formdata_rows, vec![populated_row()]);
        editor.formdata_input_states[0].type_select.clone()
    });
    cx.update(|_, cx| live_select.update(cx, |_, cx| cx.emit(SelectEvent::Confirm(Some("File")))));
    cx.update(|_, cx| {
        assert_eq!(
            editor.read(cx).get_draft(cx).formdata_rows[0].value,
            FormDataValue::File {
                path: "value".into()
            }
        );
    });
}
