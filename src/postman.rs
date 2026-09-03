//! Postman Collection v2.1 import/export.
//!
//! This module deliberately contains no GPUI or database code. It converts
//! between the persisted collection model and Postman's JSON shape so the
//! conversion can be tested independently and imported collections can be
//! validated before a database transaction starts.

use anyhow::{Result, anyhow};
use serde_json::{Value, json};

use crate::types::{
    AuthConfig, AuthType, BodyType, Collection, CollectionFolder, FormDataRow, FormDataValue,
    HeaderState, HeaderType, HttpMethod, ParamState, RawSubtype, RequestData, SavedRequest,
};

const V21_SCHEMA: &str = "https://schema.getpostman.com/json/collection/v2.1.0/collection.json";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostmanWarning {
    pub path: String,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct ImportResult {
    pub collection: Collection,
    pub warnings: Vec<PostmanWarning>,
}

fn warning(
    warnings: &mut Vec<PostmanWarning>,
    path: impl Into<String>,
    message: impl Into<String>,
) {
    warnings.push(PostmanWarning {
        path: path.into(),
        message: message.into(),
    });
}

fn text(value: Option<&Value>) -> String {
    value
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn text_entry(value: &Value, key: &str) -> String {
    text(value.get(key))
}

fn bool_entry(value: &Value, key: &str) -> bool {
    value.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn parse_method(value: &Value, path: &str, warnings: &mut Vec<PostmanWarning>) -> HttpMethod {
    let method = text(value.get("method"));
    if let Some(method) = HttpMethod::from_str(&method) {
        method
    } else {
        warning(
            warnings,
            path,
            format!("unsupported or missing HTTP method {method}, defaulted to GET"),
        );
        HttpMethod::GET
    }
}

fn parse_url(
    value: Option<&Value>,
    path: &str,
    warnings: &mut Vec<PostmanWarning>,
) -> (String, Vec<ParamState>) {
    let Some(value) = value else {
        warning(
            warnings,
            path,
            "request has no URL; imported as an empty URL",
        );
        return (String::new(), Vec::new());
    };

    if let Some(raw) = value.as_str() {
        let params = crate::url_params::parse_query_params(raw)
            .into_iter()
            .map(|(key, value)| ParamState {
                enabled: true,
                key,
                value,
            })
            .collect();
        return (raw.to_string(), params);
    }

    let raw = text(value.get("raw"));
    let params = value
        .get("query")
        .and_then(Value::as_array)
        .map(|query| {
            query
                .iter()
                .filter_map(|item| {
                    let key = text(item.get("key"));
                    if key.is_empty() && item.get("value").is_none() {
                        return None;
                    }
                    Some(ParamState {
                        enabled: !bool_entry(item, "disabled"),
                        key,
                        value: text(item.get("value")),
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|| {
            crate::url_params::parse_query_params(&raw)
                .into_iter()
                .map(|(key, value)| ParamState {
                    enabled: true,
                    key,
                    value,
                })
                .collect()
        });

    if !raw.is_empty() {
        // Store the wire URL with disabled query rows removed. The disabled
        // rows remain in `params_state` for the Params editor and are emitted
        // back to Postman on export, but the request sender must not send them.
        let base = crate::url_params::extract_base_url(&raw);
        let query_params = params
            .iter()
            .map(|param| {
                crate::url_params::QueryParam::new(&param.key, &param.value, param.enabled)
            })
            .collect::<Vec<_>>();
        return (
            crate::url_params::build_url_with_params(&base, &query_params),
            params,
        );
    }

    let protocol = text(value.get("protocol"));
    let host = value
        .get("host")
        .and_then(Value::as_array)
        .map(|parts| {
            parts
                .iter()
                .map(|part| text(Some(part)))
                .collect::<Vec<_>>()
                .join(".")
        })
        .unwrap_or_else(|| text(value.get("host")));
    let port = text(value.get("port"));
    let path_part = value
        .get("path")
        .and_then(Value::as_array)
        .map(|parts| {
            parts
                .iter()
                .map(|part| text(Some(part)))
                .collect::<Vec<_>>()
                .join("/")
        })
        .unwrap_or_else(|| text(value.get("path")));

    let mut built = String::new();
    if !protocol.is_empty() {
        built.push_str(&protocol);
        built.push_str("://");
    }
    built.push_str(&host);
    if !port.is_empty() {
        built.push(':');
        built.push_str(&port);
    }
    if !path_part.is_empty() {
        if !path_part.starts_with('/') {
            built.push('/');
        }
        built.push_str(&path_part);
    }
    if !params.is_empty() {
        built.push('?');
        built.push_str(
            &params
                .iter()
                .filter(|param| param.enabled)
                .map(|param| {
                    format!(
                        "{}={}",
                        urlencoding::encode(&param.key),
                        urlencoding::encode(&param.value)
                    )
                })
                .collect::<Vec<_>>()
                .join("&"),
        );
    }
    if built.is_empty() {
        warning(warnings, path, "could not reconstruct request URL");
    }
    (built, params)
}

fn parse_headers(
    value: Option<&Value>,
    path: &str,
    warnings: &mut Vec<PostmanWarning>,
) -> (Vec<(String, String)>, Vec<HeaderState>) {
    let mut enabled_headers = Vec::new();
    let mut states = Vec::new();
    let Some(headers) = value.and_then(Value::as_array) else {
        return (enabled_headers, states);
    };

    for (index, item) in headers.iter().enumerate() {
        let item_path = format!("{path}.header[{index}]");
        let key = text_entry(item, "key");
        let value = text_entry(item, "value");
        if key.is_empty() {
            warning(warnings, item_path, "header without a key was ignored");
            continue;
        }
        if crate::types::is_transport_owned_header(&key) {
            continue;
        }
        let enabled = !bool_entry(item, "disabled");
        let state = HeaderState {
            enabled,
            key: key.clone(),
            value: value.clone(),
            header_type: HeaderType::Custom,
            predefined: None,
        };
        if enabled {
            enabled_headers.push((key.clone(), value.clone()));
        }
        states.push(state);
    }
    (enabled_headers, states)
}

fn raw_subtype(language: &str, content_type: &str) -> RawSubtype {
    let language = language.to_ascii_lowercase();
    if language == "json" || content_type.contains("json") {
        RawSubtype::Json
    } else if language == "xml" || content_type.contains("xml") {
        RawSubtype::Xml
    } else if language == "javascript" || language == "js" || content_type.contains("javascript") {
        RawSubtype::JavaScript
    } else {
        RawSubtype::Text
    }
}

fn content_type(headers: &[(String, String)]) -> String {
    headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case("content-type"))
        .map(|(_, value)| value.to_ascii_lowercase())
        .unwrap_or_default()
}

fn parse_body(
    value: Option<&Value>,
    headers: &[(String, String)],
    path: &str,
    warnings: &mut Vec<PostmanWarning>,
) -> BodyType {
    let Some(body) = value else {
        return BodyType::None;
    };
    let mode = text(body.get("mode"));
    match mode.as_str() {
        "raw" => {
            let content = text(body.get("raw"));
            let language = body
                .get("options")
                .and_then(|options| options.get("raw"))
                .and_then(|raw| raw.get("language"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            BodyType::Raw {
                content,
                subtype: raw_subtype(language, &content_type(headers)),
            }
        }
        "formdata" => {
            let rows = body
                .get("formdata")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| {
                            let key = text_entry(item, "key");
                            if key.is_empty() {
                                return None;
                            }
                            let enabled = !bool_entry(item, "disabled");
                            let value = if text_entry(item, "type") == "file" {
                                let path = item
                                    .get("src")
                                    .and_then(|src| {
                                        src.as_str().map(str::to_string).or_else(|| {
                                            src.as_array()
                                                .and_then(|array| array.first())
                                                .and_then(Value::as_str)
                                                .map(str::to_string)
                                        })
                                    })
                                    .unwrap_or_default();
                                FormDataValue::File { path }
                            } else {
                                FormDataValue::Text(text_entry(item, "value"))
                            };
                            Some(FormDataRow {
                                enabled,
                                key,
                                value,
                            })
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            BodyType::FormData(rows)
        }
        "urlencoded" => {
            let mut pairs = Vec::new();
            if let Some(items) = body.get("urlencoded").and_then(Value::as_array) {
                for item in items {
                    let key = text_entry(item, "key");
                    if key.is_empty() {
                        continue;
                    }
                    if bool_entry(item, "disabled") {
                        warning(
                            warnings,
                            path,
                            "disabled URL-encoded fields were ignored because this app stores URL-encoded bodies as raw content",
                        );
                        continue;
                    }
                    pairs.push(format!(
                        "{}={}",
                        urlencoding::encode(&key),
                        urlencoding::encode(&text_entry(item, "value"))
                    ));
                }
            }
            BodyType::Raw {
                content: pairs.join("&"),
                subtype: RawSubtype::UrlEncoded,
            }
        }
        "file" => {
            warning(
                warnings,
                path,
                "file request bodies are not supported and were ignored",
            );
            BodyType::None
        }
        "" => BodyType::None,
        other => {
            warning(
                warnings,
                path,
                format!("unsupported body mode {other} was ignored"),
            );
            BodyType::None
        }
    }
}

fn auth_entry(auth: &Value, section: &str, key: &str) -> String {
    auth.get(section)
        .and_then(Value::as_array)
        .and_then(|entries| entries.iter().find(|entry| text_entry(entry, "key") == key))
        .map(|entry| text_entry(entry, "value"))
        .unwrap_or_default()
}

fn parse_auth(value: Option<&Value>, path: &str, warnings: &mut Vec<PostmanWarning>) -> AuthConfig {
    let Some(auth) = value else {
        return AuthConfig::default();
    };
    let kind = text(auth.get("type")).to_ascii_lowercase();
    match kind.as_str() {
        "bearer" => AuthConfig {
            auth_type: AuthType::Bearer,
            bearer_token: auth_entry(auth, "bearer", "token"),
            ..AuthConfig::default()
        },
        "basic" => AuthConfig {
            auth_type: AuthType::Basic,
            basic_username: auth_entry(auth, "basic", "username"),
            basic_password: auth_entry(auth, "basic", "password"),
            ..AuthConfig::default()
        },
        "apikey" => {
            let location = auth_entry(auth, "apikey", "in").to_ascii_lowercase();
            if !location.is_empty() && location != "header" {
                warning(
                    warnings,
                    path,
                    format!("API key location {location} is not supported; imported as a header"),
                );
            }
            AuthConfig {
                auth_type: AuthType::ApiKey,
                api_key_name: auth_entry(auth, "apikey", "key"),
                api_key_value: auth_entry(auth, "apikey", "value"),
                ..AuthConfig::default()
            }
        }
        "" | "noauth" => AuthConfig::default(),
        "inherit" => {
            warning(
                warnings,
                path,
                "inherited authentication is not supported; imported as no auth",
            );
            AuthConfig::default()
        }
        other => {
            warning(
                warnings,
                path,
                format!("unsupported auth type {other} was imported as no auth"),
            );
            AuthConfig::default()
        }
    }
}

fn parse_request(item: &Value, path: &str, warnings: &mut Vec<PostmanWarning>) -> SavedRequest {
    let request_value = item.get("request").unwrap_or(&Value::Null);
    let name = text(item.get("name"));
    let request_path = format!("{path}/request");
    let (url, params_state) = parse_url(request_value.get("url"), &request_path, warnings);
    let (headers, headers_state) =
        parse_headers(request_value.get("header"), &request_path, warnings);
    let header_values = headers_state
        .iter()
        .map(|header| (header.key.clone(), header.value.clone()))
        .collect::<Vec<_>>();
    let request = RequestData {
        method: parse_method(request_value, &request_path, warnings),
        url,
        headers,
        body: parse_body(
            request_value.get("body"),
            &header_values,
            &request_path,
            warnings,
        ),
        auth: parse_auth(request_value.get("auth"), &request_path, warnings),
    };
    SavedRequest {
        id: 0,
        collection_id: 0,
        folder_id: None,
        name: if name.is_empty() {
            "Imported Request".to_string()
        } else {
            name
        },
        request,
        params_state,
        headers_state,
        position: 0,
        created_at: String::new(),
        updated_at: String::new(),
    }
}

fn parse_items(
    items: &[Value],
    collection_id: i64,
    parent_id: Option<i64>,
    path: &str,
    warnings: &mut Vec<PostmanWarning>,
) -> (Vec<CollectionFolder>, Vec<SavedRequest>) {
    let mut folders = Vec::new();
    let mut requests = Vec::new();
    let mut folder_position = 0;
    let mut request_position = 0;
    for (index, item) in items.iter().enumerate() {
        let item_path = format!("{path}/item[{index}]");
        if let Some(children) = item.get("item").and_then(Value::as_array) {
            let name = text(item.get("name"));
            let (child_folders, child_requests) = parse_items(
                children,
                collection_id,
                Some(index as i64 + 1),
                &item_path,
                warnings,
            );
            folders.push(CollectionFolder {
                id: 0,
                collection_id,
                parent_id,
                name: if name.is_empty() {
                    "Imported Folder".to_string()
                } else {
                    name
                },
                position: folder_position,
                folders: child_folders,
                requests: child_requests,
            });
            folder_position += 1;
        } else if item.get("request").is_some() {
            let mut request = parse_request(item, &item_path, warnings);
            request.collection_id = collection_id;
            request.folder_id = parent_id;
            request.position = request_position;
            requests.push(request);
            request_position += 1;
        } else {
            warning(
                warnings,
                item_path,
                "item has neither a request nor child items and was ignored",
            );
        }
    }
    (folders, requests)
}

pub fn import_collection(json_text: &str) -> Result<ImportResult> {
    let root: Value = serde_json::from_str(json_text)?;
    let schema = root
        .get("info")
        .and_then(|info| info.get("schema"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    if schema != V21_SCHEMA {
        return Err(anyhow!(
            "unsupported Postman Collection schema; expected v2.1"
        ));
    }

    let mut warnings = Vec::new();
    if root.get("variable").is_some() {
        warning(
            &mut warnings,
            "variable",
            "collection variables are not imported; use Environment instead",
        );
    }
    for field in ["event", "auth"] {
        if root.get(field).is_some() {
            warning(
                &mut warnings,
                field,
                format!("collection-level {field} is not imported"),
            );
        }
    }

    let name = text(root.get("info").and_then(|info| info.get("name")));
    let items = root
        .get("item")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("Postman collection has no item array"))?;
    let (folders, requests) = parse_items(items, 0, None, "item", &mut warnings);
    Ok(ImportResult {
        collection: Collection {
            id: 0,
            name: if name.is_empty() {
                "Imported Collection".to_string()
            } else {
                name
            },
            position: 0,
            folders,
            requests,
        },
        warnings,
    })
}

fn export_headers(saved: &SavedRequest) -> Vec<Value> {
    if !saved.headers_state.is_empty() {
        return saved
            .headers_state
            .iter()
            .filter(|header| !header.key.trim().is_empty() && !header.is_transport_owned())
            .map(|header| {
                json!({
                    "key": header.key,
                    "value": header.value,
                    "disabled": !header.enabled,
                })
            })
            .collect();
    }
    saved
        .request
        .headers
        .iter()
        .filter(|(key, _)| !key.trim().is_empty() && !crate::types::is_transport_owned_header(key))
        .map(|(key, value)| json!({ "key": key, "value": value }))
        .collect()
}

fn export_url(saved: &SavedRequest) -> Value {
    let params = if saved.params_state.is_empty() {
        crate::url_params::parse_query_params(&saved.request.url)
            .into_iter()
            .map(|(key, value)| ParamState {
                enabled: true,
                key,
                value,
            })
            .collect::<Vec<_>>()
    } else {
        saved.params_state.clone()
    };
    json!({
        "raw": saved.request.url,
        "query": params.into_iter().filter(|param| !param.key.trim().is_empty()).map(|param| json!({
            "key": param.key,
            "value": param.value,
            "disabled": !param.enabled,
        })).collect::<Vec<_>>()
    })
}

fn export_body(body: &BodyType) -> Option<Value> {
    match body {
        BodyType::None => None,
        BodyType::Raw { content, subtype } => match subtype {
            RawSubtype::UrlEncoded => {
                let fields = content
                    .split('&')
                    .filter(|field| !field.is_empty())
                    .map(|field| {
                        let (key, value) = field.split_once('=').unwrap_or((field, ""));
                        json!({
                            "key": urlencoding::decode(key).unwrap_or_else(|_| key.into()).to_string(),
                            "value": urlencoding::decode(value).unwrap_or_else(|_| value.into()).to_string(),
                            "type": "text",
                        })
                    })
                    .collect::<Vec<_>>();
                Some(json!({ "mode": "urlencoded", "urlencoded": fields }))
            }
            subtype => {
                let language = match subtype {
                    RawSubtype::Json => "json",
                    RawSubtype::Xml => "xml",
                    RawSubtype::JavaScript => "javascript",
                    RawSubtype::Text | RawSubtype::UrlEncoded => "text",
                };
                Some(json!({
                    "mode": "raw",
                    "raw": content,
                    "options": { "raw": { "language": language } },
                }))
            }
        },
        BodyType::FormData(rows) => Some(json!({
            "mode": "formdata",
            "formdata": rows.iter().filter(|row| !row.key.trim().is_empty()).map(|row| match &row.value {
                FormDataValue::Text(value) => json!({ "key": row.key, "value": value, "type": "text", "disabled": !row.enabled }),
                FormDataValue::File { path } => json!({ "key": row.key, "src": path, "type": "file", "disabled": !row.enabled }),
            }).collect::<Vec<_>>(),
        })),
    }
}

fn export_auth(auth: &AuthConfig) -> Option<Value> {
    match auth.auth_type {
        AuthType::None => None,
        AuthType::Bearer => Some(json!({
            "type": "bearer",
            "bearer": [{ "key": "token", "value": auth.bearer_token, "type": "string" }],
        })),
        AuthType::Basic => Some(json!({
            "type": "basic",
            "basic": [
                { "key": "username", "value": auth.basic_username, "type": "string" },
                { "key": "password", "value": auth.basic_password, "type": "string" },
            ],
        })),
        AuthType::ApiKey => Some(json!({
            "type": "apikey",
            "apikey": [
                { "key": "key", "value": auth.api_key_name, "type": "string" },
                { "key": "value", "value": auth.api_key_value, "type": "string" },
                { "key": "in", "value": "header", "type": "string" },
            ],
        })),
    }
}

fn export_request(saved: &SavedRequest) -> Value {
    let mut request = json!({
        "method": saved.request.method.as_str(),
        "header": export_headers(saved),
        "url": export_url(saved),
    });
    if let Some(body) = export_body(&saved.request.body) {
        request["body"] = body;
    }
    if let Some(auth) = export_auth(&saved.request.auth) {
        request["auth"] = auth;
    }
    json!({ "name": saved.name, "request": request })
}

fn export_folder(folder: &CollectionFolder) -> Value {
    let mut items = Vec::new();
    for request in &folder.requests {
        items.push(export_request(request));
    }
    for child in &folder.folders {
        items.push(export_folder(child));
    }
    json!({ "name": folder.name, "item": items })
}

pub fn export_collection(collection: &Collection) -> Result<String> {
    let mut items = Vec::new();
    for request in &collection.requests {
        items.push(export_request(request));
    }
    for folder in &collection.folders {
        items.push(export_folder(folder));
    }
    Ok(serde_json::to_string_pretty(&json!({
        "info": { "name": collection.name, "schema": V21_SCHEMA },
        "item": items,
    }))?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> &'static str {
        r#"
        {
          "info": {
            "name": "Demo API",
            "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json"
          },
          "variable": [{"key":"ignored","value":"x"}],
          "item": [
            {
              "name": "Users",
              "item": [
                {
                  "name": "List users",
                  "request": {
                    "method": "GET",
                    "header": [
                      {"key":"Accept","value":"application/json"},
                      {"key":"X-Debug","value":"1","disabled":true}
                    ],
                    "url": {
                      "raw":"https://api.example.com/users?page=1&archived=false",
                      "query":[
                        {"key":"page","value":"1"},
                        {"key":"archived","value":"false","disabled":true}
                      ]
                    },
                    "auth": {"type":"bearer","bearer":[{"key":"token","value":"abc"}]}
                  }
                }
              ]
            }
          ]
        }
        "#
    }

    #[test]
    fn imports_nested_tree_and_disabled_state() {
        let result = import_collection(fixture()).unwrap();
        assert_eq!(result.collection.name, "Demo API");
        assert_eq!(result.collection.folders[0].name, "Users");
        let request = &result.collection.folders[0].requests[0];
        assert_eq!(request.name, "List users");
        assert_eq!(request.request.auth.auth_type, AuthType::Bearer);
        assert_eq!(request.headers_state.len(), 2);
        assert!(!request.headers_state[1].enabled);
        assert!(!request.params_state[1].enabled);
        assert!(!result.warnings.is_empty());
    }

    #[test]
    fn import_and_export_strip_content_length() {
        let input = json!({
            "info": {"name":"Headers", "schema": V21_SCHEMA},
            "item": [{
                "name": "request",
                "request": {
                    "method": "POST",
                    "url": "https://example.test",
                    "header": [
                        {"key":"Content-Length", "value":"1"},
                        {"key":"X-Trace", "value":"kept", "disabled":true}
                    ],
                    "body": {"mode":"raw", "raw":"你好"}
                }
            }]
        });

        let imported = import_collection(&input.to_string()).unwrap();
        let saved = &imported.collection.requests[0];
        assert!(saved.request.headers.is_empty());
        assert_eq!(saved.headers_state.len(), 1);
        assert_eq!(saved.headers_state[0].key, "X-Trace");
        assert!(!saved.headers_state[0].enabled);

        let exported: Value =
            serde_json::from_str(&export_collection(&imported.collection).unwrap()).unwrap();
        let headers = exported["item"][0]["request"]["header"].as_array().unwrap();
        assert_eq!(headers.len(), 1);
        assert_eq!(headers[0]["key"], "X-Trace");
    }

    #[test]
    fn rejects_non_v21_schema() {
        let result = import_collection(r#"{"info":{"name":"x","schema":"v2.0"},"item":[]}"#);
        assert!(result.is_err());
    }

    #[test]
    fn exports_v21_collection_and_roundtrips_core_fields() {
        let imported = import_collection(fixture()).unwrap();
        let exported = export_collection(&imported.collection).unwrap();
        let value: Value = serde_json::from_str(&exported).unwrap();
        assert_eq!(value["info"]["schema"], V21_SCHEMA);
        assert_eq!(value["item"][0]["name"], "Users");
        assert_eq!(value["item"][0]["item"][0]["request"]["method"], "GET");
        assert_eq!(
            value["item"][0]["item"][0]["request"]["auth"]["type"],
            "bearer"
        );
    }

    #[test]
    fn maps_urlencoded_and_formdata_bodies() {
        let input = json!({
            "info": {"name":"Bodies", "schema": V21_SCHEMA},
            "item": [
                {"name":"encoded", "request":{"method":"POST","url":"https://x","body":{"mode":"urlencoded","urlencoded":[{"key":"a","value":"1"}]}}},
                {"name":"form", "request":{"method":"POST","url":"https://x","body":{"mode":"formdata","formdata":[{"key":"file","type":"file","src":"/tmp/a.txt"}]}}}
            ]
        });
        let result = import_collection(&input.to_string()).unwrap();
        assert!(matches!(
            result.collection.requests[0].request.body,
            BodyType::Raw {
                subtype: RawSubtype::UrlEncoded,
                ..
            }
        ));
        assert!(matches!(
            result.collection.requests[1].request.body,
            BodyType::FormData(_)
        ));
        let output: Value =
            serde_json::from_str(&export_collection(&result.collection).unwrap()).unwrap();
        assert_eq!(output["item"][0]["request"]["body"]["mode"], "urlencoded");
        assert_eq!(output["item"][1]["request"]["body"]["mode"], "formdata");
    }

    #[test]
    fn disabled_query_is_not_left_in_the_wire_url() {
        let input = json!({
            "info": {"name":"Query", "schema": V21_SCHEMA},
            "item": [{
                "name": "request",
                "request": {
                    "method": "GET",
                    "url": {
                        "raw": "https://example.test/items?page=1&debug=true#results",
                        "query": [
                            {"key":"page", "value":"1"},
                            {"key":"debug", "value":"true", "disabled":true}
                        ]
                    }
                }
            }]
        });
        let result = import_collection(&input.to_string()).unwrap();
        let request = &result.collection.requests[0];
        assert_eq!(
            request.request.url,
            "https://example.test/items?page=1#results"
        );
        assert_eq!(request.params_state.len(), 2);
        assert!(!request.params_state[1].enabled);
        let output: Value =
            serde_json::from_str(&export_collection(&result.collection).unwrap()).unwrap();
        assert_eq!(
            output["item"][0]["request"]["url"]["raw"],
            "https://example.test/items?page=1#results"
        );
        assert_eq!(
            output["item"][0]["request"]["url"]["query"][1]["disabled"],
            true
        );
    }

    #[test]
    fn imports_basic_and_api_key_auth_fields() {
        let input = json!({
            "info": {"name":"Auth", "schema": V21_SCHEMA},
            "item": [
                {"name":"basic", "request":{"method":"GET", "url":"https://x", "auth":{"type":"basic", "basic":[{"key":"username","value":"u"},{"key":"password","value":"p"}]}}},
                {"name":"key", "request":{"method":"GET", "url":"https://x", "auth":{"type":"apikey", "apikey":[{"key":"key","value":"X-Key"},{"key":"value","value":"secret"},{"key":"in","value":"header"}]}}}
            ]
        });
        let result = import_collection(&input.to_string()).unwrap();
        assert_eq!(
            result.collection.requests[0].request.auth.auth_type,
            AuthType::Basic
        );
        assert_eq!(
            result.collection.requests[0].request.auth.basic_username,
            "u"
        );
        assert_eq!(
            result.collection.requests[0].request.auth.basic_password,
            "p"
        );
        assert_eq!(
            result.collection.requests[1].request.auth.auth_type,
            AuthType::ApiKey
        );
        assert_eq!(
            result.collection.requests[1].request.auth.api_key_name,
            "X-Key"
        );
        assert_eq!(
            result.collection.requests[1].request.auth.api_key_value,
            "secret"
        );
        assert!(result.warnings.is_empty());
    }
}
