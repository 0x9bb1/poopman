use super::AuthEditor;
use gpui::{AppContext as _, TestAppContext};

#[gpui::test]
fn editing_each_auth_input_notifies_its_owner(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let cx = cx.add_empty_window();
    let editor = cx.update(|window, cx| cx.new(|cx| AuthEditor::new(window, cx)));
    let changes = cx.update(|_, cx| {
        cx.new(|cx| {
            cx.observe(&editor, |count: &mut usize, _, _| *count += 1)
                .detach();
            0usize
        })
    });
    for field in 0..5 {
        let before = cx.update(|_, cx| *changes.read(cx));
        cx.update(|window, cx| {
            editor.update(cx, |editor, cx| {
                let input = match field {
                    0 => &editor.bearer_token,
                    1 => &editor.basic_username,
                    2 => &editor.basic_password,
                    3 => &editor.api_key_name,
                    _ => &editor.api_key_value,
                };
                input.update(cx, |input, cx| input.set_value("edited", window, cx));
            })
        });
        cx.run_until_parked();
        cx.update(|_, cx| assert!(*changes.read(cx) > before));
    }
}
