use super::save_tests::setup;
use crate::db::Database;
use crate::types::{AuthConfig, BodyType, HttpMethod, RequestData};
use std::sync::Arc;

/// Postman reports a malformed URL like a transport failure: the response
/// pane shows the error and the attempt is still logged to History.
#[gpui::test]
fn invalid_url_is_reported_in_the_response_and_logged(cx: &mut gpui::TestAppContext) {
    let db = Arc::new(Database::new_in_memory());
    let (app, cx) = setup(cx, db.clone());

    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.request_editor.update(cx, |editor, cx| {
                let request = RequestData {
                    method: HttpMethod::GET,
                    url: "http://".into(),
                    headers: vec![],
                    body: BodyType::None,
                    auth: AuthConfig::default(),
                };
                editor.load_request(&request, window, cx);
                editor.send(window, cx);
            })
        })
    });
    cx.run_until_parked();

    cx.update(|_, cx| {
        let app = app.read(cx);
        let response = app.request_tabs[0]
            .response
            .clone()
            .expect("error response");
        assert_eq!(response.status, None);
        assert_eq!(response.body, b"Could not send request: invalid URL");
        assert_eq!(
            app.response_viewer
                .read(cx)
                .get_response()
                .map(|r| r.status),
            Some(None)
        );
    });
    let history = db.load_recent_history(10).unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].request.url, "http://");
}
