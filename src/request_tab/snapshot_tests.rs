use super::*;
use crate::types::{AuthType, BodyKind, FormDataRow, FormDataValue, RawSubtype};

#[test]
fn every_editor_field_protects_a_scratch_tab() {
    let mut cases = Vec::new();
    let mut tab = RequestTab::new_empty(0);
    tab.request.method = HttpMethod::POST;
    cases.push(tab);
    let mut tab = RequestTab::new_empty(0);
    tab.request.auth.auth_type = AuthType::Bearer;
    tab.request.auth.bearer_token = "draft".into();
    cases.push(tab);
    let mut tab = RequestTab::new_empty(0);
    tab.request.headers.push(("X-Draft".into(), "yes".into()));
    cases.push(tab);
    let mut tab = RequestTab::new_empty(0);
    tab.params_state = Some(vec![ParamState {
        enabled: false,
        key: "draft".into(),
        value: "yes".into(),
    }]);
    cases.push(tab);
    let mut tab = RequestTab::new_empty(0);
    tab.headers_state = Some(tab.snapshot().headers_state);
    tab.headers_state.as_mut().unwrap()[0].enabled = false;
    cases.push(tab);
    let mut tab = RequestTab::new_empty(0);
    tab.request.body = BodyType::Raw {
        content: " ".into(),
        subtype: RawSubtype::Json,
    };
    cases.push(tab);
    let mut tab = RequestTab::new_empty(0);
    tab.body_draft.raw_subtype = RawSubtype::Xml;
    cases.push(tab);
    let mut tab = RequestTab::new_empty(0);
    tab.body_draft.kind = BodyKind::None;
    tab.body_draft.raw_content = "inactive draft".into();
    cases.push(tab);
    let mut tab = RequestTab::new_empty(0);
    tab.body_draft.formdata_rows.push(FormDataRow {
        enabled: false,
        key: "file".into(),
        value: FormDataValue::File {
            path: "draft.txt".into(),
        },
    });
    cases.push(tab);
    for (index, tab) in cases.iter().enumerate() {
        assert!(tab.is_dirty(), "case {index}");
        assert!(!tab.is_blank(), "case {index}");
    }
}

#[test]
fn placeholders_and_default_headers_are_canonical() {
    let mut tab = RequestTab::new_empty(0);
    let mut headers = tab.snapshot().headers_state;
    headers.push(HeaderState {
        enabled: true,
        key: String::new(),
        value: String::new(),
        header_type: HeaderType::Custom,
        predefined: None,
    });
    tab.headers_state = Some(headers);
    tab.params_state = Some(vec![ParamState {
        enabled: true,
        key: String::new(),
        value: String::new(),
    }]);
    tab.body_draft.formdata_rows.push(FormDataRow {
        enabled: true,
        key: String::new(),
        value: FormDataValue::Text(String::new()),
    });
    assert!(!tab.is_dirty());
    assert!(tab.is_blank());
}

#[test]
fn reverting_changes_clears_dirty_state() {
    let mut tab = RequestTab::new_empty(0);
    tab.request.method = HttpMethod::POST;
    assert!(tab.is_dirty());
    tab.request.method = HttpMethod::GET;
    assert!(!tab.is_dirty());
    assert!(tab.is_blank());
}

#[test]
fn saving_an_older_snapshot_keeps_later_edits_dirty() {
    let mut tab = RequestTab::new_empty(0);
    tab.request.url = "https://example.test/first".into();
    let submitted = tab.snapshot();
    tab.request.url = "https://example.test/later".into();
    tab.mark_saved(submitted);
    assert!(tab.is_dirty());
    tab.request.url = "https://example.test/first".into();
    assert!(!tab.is_dirty());
    assert!(!tab.is_blank());
}

#[test]
fn response_and_request_runtime_do_not_make_a_request_dirty() {
    let mut tab = RequestTab::new_empty(0);
    tab.active_request_id = Some(1);
    assert!(!tab.is_dirty());
    assert!(!tab.is_blank());
}
