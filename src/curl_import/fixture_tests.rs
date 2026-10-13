use super::*;
use serde_json::Value;

#[test]
fn curl_import_fixtures() {
    let fixtures: Vec<Value> =
        serde_json::from_str(include_str!("../../tests/fixtures/import/curl.json")).unwrap();
    let mut failures = Vec::new();
    for case in fixtures {
        let name = case["name"].as_str().unwrap();
        let actual = parse_curl(case["command"].as_str().unwrap());
        let result = std::panic::catch_unwind(|| {
            if let Some(expected) = case["error"].as_str() {
                let error = actual.expect_err(name).to_string();
                assert!(error.to_lowercase().contains(expected), "{name}: {error}");
                assert!(
                    !error.contains("payload.txt"),
                    "error must not echo local paths"
                );
                return;
            }
            let request = actual.unwrap().unwrap();
            assert_eq!(request.method.as_str(), case["method"].as_str().unwrap());
            if let Some(url) = case["url"].as_str() {
                assert_eq!(request.url, url);
            }
            match case["body"].as_str() {
                None => assert_eq!(request.body, BodyType::None),
                Some(expected) => {
                    let BodyType::Raw { content, subtype } = &request.body else {
                        panic!("{name}: expected raw body")
                    };
                    assert_eq!(content, expected);
                    assert_eq!(
                        serde_json::to_value(subtype).unwrap(),
                        case.get("subtype")
                            .cloned()
                            .unwrap_or(Value::String("UrlEncoded".into()))
                    );
                    assert!(request.headers.iter().any(|(key, value)| {
                        key.eq_ignore_ascii_case("content-type")
                            && value
                                == case["content_type"]
                                    .as_str()
                                    .unwrap_or("application/x-www-form-urlencoded")
                    }));
                }
            }
        });
        if result.is_err() {
            failures.push(name.to_owned());
        }
    }
    assert!(failures.is_empty(), "failed fixtures: {failures:?}");
}
