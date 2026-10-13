use super::*;

const AUTH_FIXTURE: &str = include_str!("../../tests/fixtures/import/postman-auth.json");

#[test]
fn postman_auth_inheritance_fixture() {
    let result = import_collection(AUTH_FIXTURE).unwrap();
    for request in &result.collection.requests[..3] {
        assert_eq!(
            request.request.auth.auth_type,
            AuthType::Bearer,
            "{}",
            request.name
        );
        assert_eq!(request.request.auth.bearer_token, "{{token}}");
    }
    assert_eq!(
        result.collection.requests[3].request.auth.auth_type,
        AuthType::None
    );
    let basic = &result.collection.folders[0];
    for request in [&basic.requests[0], &basic.folders[0].requests[0]] {
        assert_eq!(request.request.auth.auth_type, AuthType::Basic);
        assert_eq!(request.request.auth.basic_username, "user");
        assert_eq!(request.request.auth.basic_password, "pass");
    }
    let noauth = &result.collection.folders[1];
    assert_eq!(noauth.requests[0].request.auth.auth_type, AuthType::None);
    assert_eq!(noauth.requests[1].request.auth.bearer_token, "local");
    let roundtrip = import_collection(&export_collection(&result.collection).unwrap()).unwrap();
    fn auths(requests: &[SavedRequest], folders: &[CollectionFolder]) -> Vec<(String, AuthConfig)> {
        let mut values: Vec<_> = requests
            .iter()
            .map(|saved| (saved.name.clone(), saved.request.auth.clone()))
            .collect();
        for folder in folders {
            values.extend(auths(&folder.requests, &folder.folders));
        }
        values
    }
    assert_eq!(
        auths(
            &roundtrip.collection.requests,
            &roundtrip.collection.folders
        ),
        auths(&result.collection.requests, &result.collection.folders)
    );
}

#[test]
fn postman_query_api_key_fixture() {
    let result = import_collection(AUTH_FIXTURE).unwrap();
    let request = &result.collection.folders[0].requests[1].request;
    let vars = std::collections::HashMap::from([("key".into(), "a b&c".into())]);
    let resolved = crate::variables::substitute_request(request, &vars);
    assert_eq!(resolved.auth.compute_header(), None);
    assert_eq!(
        crate::types::effective_wire_headers(&resolved.headers, &resolved.auth),
        request.headers
    );
    for target in crate::code_gen::CodeTarget::all() {
        let code = crate::code_gen::generate(target, &resolved);
        assert!(
            code.contains("https://example.test?q=1&API-KEY=keep&api-key=a%20b%26c#results"),
            "{target:?}: {code}"
        );
        assert!(!code.contains("api-key=stale"));
    }
    assert_eq!(request.history_snapshot().auth, request.auth);
    let saved: RequestData =
        serde_json::from_str(&serde_json::to_string(request).unwrap()).unwrap();
    assert_eq!(saved, *request);
    let exported: Value =
        serde_json::from_str(&export_collection(&result.collection).unwrap()).unwrap();
    let auth = &exported["item"][4]["item"][1]["request"]["auth"];
    assert!(
        auth["apikey"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["key"] == "in" && entry["value"] == "query")
    );
}

#[test]
fn curl_urlencoded_fixtures_survive_postman_export() {
    let cases: Vec<Value> =
        serde_json::from_str(include_str!("../../tests/fixtures/import/curl.json")).unwrap();
    for case in cases.into_iter().filter(|case| case.get("error").is_none()) {
        let request = crate::curl_import::parse_curl(case["command"].as_str().unwrap())
            .unwrap()
            .unwrap();
        let BodyType::Raw {
            content,
            subtype: RawSubtype::UrlEncoded,
        } = &request.body
        else {
            continue;
        };
        let expected = url::form_urlencoded::parse(content.as_bytes())
            .into_owned()
            .collect::<Vec<_>>();
        let mut imported = import_collection(AUTH_FIXTURE).unwrap();
        imported.collection.requests[0].request = request;
        let roundtrip =
            import_collection(&export_collection(&imported.collection).unwrap()).unwrap();
        let BodyType::Raw { content, .. } = &roundtrip.collection.requests[0].request.body else {
            panic!("missing body")
        };
        assert_eq!(
            url::form_urlencoded::parse(content.as_bytes())
                .into_owned()
                .collect::<Vec<_>>(),
            expected,
            "{}",
            case["name"]
        );
    }
}

#[test]
fn legacy_auth_defaults_to_header_location() {
    let auth: AuthConfig = serde_json::from_value(json!({
        "auth_type":"ApiKey", "bearer_token":"", "basic_username":"", "basic_password":"",
        "api_key_name":"X-Key", "api_key_value":"secret"
    }))
    .unwrap();
    assert_eq!(auth.api_key_location, ApiKeyLocation::Header);
    assert_eq!(
        auth.compute_header(),
        Some(("X-Key".into(), "secret".into()))
    );
    assert_eq!(
        auth.effective_url("https://example.test"),
        "https://example.test"
    );
}

#[test]
fn postman_body_fixtures() {
    let result = import_collection(include_str!(
        "../../tests/fixtures/import/postman-bodies.json"
    ))
    .unwrap();
    let requests = &result.collection.requests;
    assert_eq!(
        requests[0].request.body,
        BodyType::Raw {
            content: "{\"token\":\"{{token}}\"}".into(),
            subtype: RawSubtype::Json
        }
    );
    assert_eq!(
        requests[1].request.body,
        BodyType::Raw {
            content: "q=a%20b%26%E4%BD%A0%E5%A5%BD".into(),
            subtype: RawSubtype::UrlEncoded
        }
    );
    let BodyType::FormData(rows) = &requests[2].request.body else {
        panic!("expected multipart")
    };
    assert_eq!(rows.len(), 3);
    assert_eq!(
        rows[1].value,
        FormDataValue::File {
            path: "a.txt".into()
        }
    );
    assert!(!rows[2].enabled);
    assert_eq!(requests[3].request.body, BodyType::None);
    assert_eq!(requests[4].request.method, HttpMethod::GET);
    assert_eq!(
        requests[5]
            .request
            .auth
            .effective_url(&requests[5].request.url),
        "https://example.test?token="
    );
    for request in &requests[..2] {
        assert!(
            request
                .request
                .headers
                .iter()
                .any(|(key, _)| key.eq_ignore_ascii_case("Content-Type")),
            "body content type must survive loading saved header state"
        );
    }
    assert!(
        result
            .warnings
            .iter()
            .any(|w| w.message.contains("Disabled script"))
    );
}

#[test]
fn postman_rejected_fixtures() {
    let cases: Vec<Value> = serde_json::from_str(include_str!(
        "../../tests/fixtures/import/postman-rejected.json"
    ))
    .unwrap();
    let mut failures = Vec::new();
    for case in cases {
        let mut root = json!({"info":{"name":"Rejected", "schema":V21_SCHEMA}, "item":[]});
        if let Some(fields) = case["root"].as_object() {
            root.as_object_mut().unwrap().extend(fields.clone());
        }
        if let Some(item) = case.get("item") {
            root["item"] = json!([item]);
        }
        if let Some(request) = case.get("request") {
            root["item"] = json!([{"name":"Rejected", "request":request}]);
        }
        match import_collection(&root.to_string()) {
            Err(error) if error.to_string().contains(case["error"].as_str().unwrap()) => {}
            result => failures.push(format!("{}: {result:?}", case["name"])),
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
