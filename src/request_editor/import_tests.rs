use super::RequestEditor;
use crate::types::{AppSettings, HeaderState, RequestData};
use gpui::{AppContext as _, TestAppContext, VisualTestContext};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc;
use std::sync::{Arc, RwLock};
use std::time::Duration;

fn capture(mut stream: TcpStream) -> String {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 4096];
    let head_end = loop {
        let count = stream.read(&mut chunk).unwrap();
        assert!(count > 0);
        bytes.extend_from_slice(&chunk[..count]);
        if let Some(index) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
            break index + 4;
        }
    };
    let head = String::from_utf8(bytes[..head_end].to_vec()).unwrap();
    let length = head
        .lines()
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .map(|(_, value)| value.trim().parse::<usize>().unwrap())
        .unwrap_or(0);
    while bytes.len() < head_end + length {
        let count = stream.read(&mut chunk).unwrap();
        assert!(count > 0);
        bytes.extend_from_slice(&chunk[..count]);
    }
    stream
        .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
        .unwrap();
    String::from_utf8(bytes).unwrap()
}

fn send_imported(
    cx: &mut VisualTestContext,
    mut request: RequestData,
    headers: Option<&[HeaderState]>,
) -> (String, RequestData) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    request.url = request.url.replace(
        "https://example.test",
        &format!("http://{}", listener.local_addr().unwrap()),
    );
    let (tx, rx) = mpsc::channel();
    let server = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        tx.send(capture(stream)).unwrap();
    });
    let editor = cx.update(|window, cx| {
        cx.new(|cx| RequestEditor::new(Arc::new(RwLock::new(AppSettings::default())), window, cx))
    });
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.load_request(&request, window, cx);
            if let Some(headers) = headers {
                editor.load_headers_state(headers, window, cx);
            }
            editor.set_env_vars(
                std::collections::HashMap::from([("key".into(), "a b&c".into())]),
                window,
                cx,
            );
        })
    });
    cx.run_until_parked();
    let snapshot = cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            let snapshot = editor.get_current_request_data(cx);
            editor.send(window, cx);
            snapshot
        })
    });
    let captured = rx.recv_timeout(Duration::from_secs(10)).unwrap();
    server.join().unwrap();
    cx.run_until_parked();
    (captured, snapshot)
}

#[gpui::test]
fn curl_fixtures_survive_editor_and_match_wire(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let cx = cx.add_empty_window();
    let cases: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("../../tests/fixtures/import/curl.json")).unwrap();
    for case in cases.into_iter().filter(|case| case.get("error").is_none()) {
        let request = crate::curl_import::parse_curl(case["command"].as_str().unwrap())
            .unwrap()
            .unwrap();
        let expected_url = case["url"].as_str().unwrap_or("https://example.test");
        let parsed = url::Url::parse(expected_url).unwrap();
        let (wire, _) = send_imported(cx, request, None);
        let first_line: Vec<_> = wire.lines().next().unwrap().split_whitespace().collect();
        assert_eq!(first_line[0], case["method"].as_str().unwrap());
        let wire_url = url::Url::parse(&format!("http://localhost{}", first_line[1])).unwrap();
        assert_eq!(wire_url.path(), parsed.path());
        assert_eq!(
            wire_url.query_pairs().collect::<Vec<_>>(),
            parsed.query_pairs().collect::<Vec<_>>(),
            "{}: {wire}",
            case["name"]
        );
        assert_eq!(
            wire.split_once("\r\n\r\n").unwrap().1,
            case["body"].as_str().unwrap_or("")
        );
        if !case["body"].is_null() || case.get("content_type").is_some() {
            let content_type = case["content_type"]
                .as_str()
                .unwrap_or("application/x-www-form-urlencoded");
            assert!(
                wire.to_lowercase()
                    .contains(&format!("content-type: {content_type}\r\n")),
                "{}: {wire}",
                case["name"]
            );
        }
    }
}

#[gpui::test]
fn postman_query_key_survives_editor_and_is_only_in_wire_query(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let cx = cx.add_empty_window();
    let imported = crate::postman::import_collection(include_str!(
        "../../tests/fixtures/import/postman-auth.json"
    ))
    .unwrap();
    let saved = &imported.collection.folders[0].requests[1];
    let (wire, snapshot) = send_imported(cx, saved.request.clone(), Some(&saved.headers_state));
    assert!(
        wire.starts_with("GET /?q=1&API-KEY=keep&api-key=a%20b%26c HTTP/1.1\r\n"),
        "{wire}"
    );
    assert!(wire.to_lowercase().contains("api-key: manual\r\n"));
    assert_eq!(snapshot.auth, saved.request.auth);
    assert!(snapshot.url.contains("api-key=stale"));
    assert!(!snapshot.url.contains("a%20b"));
}

#[gpui::test]
fn postman_body_content_type_survives_saved_headers(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let cx = cx.add_empty_window();
    let imported = crate::postman::import_collection(include_str!(
        "../../tests/fixtures/import/postman-bodies.json"
    ))
    .unwrap();
    for (saved, content_type) in imported.collection.requests[..2]
        .iter()
        .zip(["application/json", "application/x-www-form-urlencoded"])
    {
        let (wire, _) = send_imported(cx, saved.request.clone(), Some(&saved.headers_state));
        assert!(
            wire.to_lowercase()
                .contains(&format!("content-type: {content_type}\r\n")),
            "{wire}"
        );
    }
}
