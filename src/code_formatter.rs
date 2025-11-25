//! Code formatting and validation utilities for request body editor.
//!
//! This module provides pure functions for formatting and validating JSON, XML,
//! and JavaScript code. All functions are stateless and can be tested independently.

/// Format JSON string with pretty indentation.
///
/// # Arguments
/// * `input` - Raw JSON string
///
/// # Returns
/// * `Ok(String)` - Formatted JSON with 2-space indentation
/// * `Err(String)` - Parse error message
///
/// # Examples
/// ```
/// let formatted = format_json(r#"{"key":"value"}"#)?;
/// assert_eq!(formatted, "{\n  \"key\": \"value\"\n}");
/// ```
pub fn format_json(input: &str) -> Result<String, String> {
    if input.trim().is_empty() {
        return Err("Empty input".to_string());
    }

    let value: serde_json::Value = serde_json::from_str(input)
        .map_err(|e| format!("JSON parse error: {}", e))?;

    serde_json::to_string_pretty(&value)
        .map_err(|e| format!("JSON format error: {}", e))
}

/// Validate JSON syntax without formatting.
///
/// # Arguments
/// * `input` - Raw JSON string to validate
///
/// # Returns
/// * `Ok(())` - Valid JSON
/// * `Err(String)` - Validation error message
pub fn validate_json(input: &str) -> Result<(), String> {
    if input.trim().is_empty() {
        return Ok(()); // Empty is considered valid (no content to validate)
    }

    serde_json::from_str::<serde_json::Value>(input)
        .map(|_| ())
        .map_err(|e| format!("Invalid JSON: {}", e))
}

/// Format XML string with indentation.
///
/// # Arguments
/// * `input` - Raw XML string
///
/// # Returns
/// * `Ok(String)` - Formatted XML with 2-space indentation
/// * `Err(String)` - Parse/format error message
pub fn format_xml(input: &str) -> Result<String, String> {
    if input.trim().is_empty() {
        return Err("Empty input".to_string());
    }

    // First validate XML syntax
    validate_xml(input)?;

    // Simple XML formatting using quick-xml
    use quick_xml::events::Event;
    use quick_xml::reader::Reader;
    use quick_xml::writer::Writer;
    use std::io::Cursor;

    let mut reader = Reader::from_str(input);
    reader.config_mut().trim_text(true);

    let mut writer = Writer::new_with_indent(Cursor::new(Vec::new()), b' ', 2);

    loop {
        match reader.read_event() {
            Ok(Event::Eof) => break,
            Ok(event) => {
                writer
                    .write_event(event)
                    .map_err(|e| format!("XML write error: {}", e))?;
            }
            Err(e) => return Err(format!("XML read error: {}", e)),
        }
    }

    let result = writer.into_inner().into_inner();
    String::from_utf8(result).map_err(|e| format!("UTF-8 conversion error: {}", e))
}

/// Validate XML syntax without formatting.
///
/// # Arguments
/// * `input` - Raw XML string to validate
///
/// # Returns
/// * `Ok(())` - Valid XML
/// * `Err(String)` - Validation error message
pub fn validate_xml(input: &str) -> Result<(), String> {
    if input.trim().is_empty() {
        return Ok(()); // Empty is considered valid
    }

    use quick_xml::events::Event;
    use quick_xml::reader::Reader;

    let mut reader = Reader::from_str(input);
    reader.config_mut().trim_text(true);

    loop {
        match reader.read_event() {
            Ok(Event::Eof) => break,
            Ok(_) => continue,
            Err(e) => return Err(format!("Invalid XML: {}", e)),
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ============ JSON Format Tests ============

    #[test]
    fn test_format_json_simple() {
        let input = r#"{"key":"value"}"#;
        let result = format_json(input).unwrap();
        assert!(result.contains("\"key\""));
        assert!(result.contains("\"value\""));
        assert!(result.contains('\n')); // Has newlines
    }

    #[test]
    fn test_format_json_nested() {
        let input = r#"{"outer":{"inner":"value"}}"#;
        let result = format_json(input).unwrap();
        assert!(result.contains("\"outer\""));
        assert!(result.contains("\"inner\""));
    }

    #[test]
    fn test_format_json_array() {
        let input = r#"[1,2,3]"#;
        let result = format_json(input).unwrap();
        assert!(result.contains("1"));
        assert!(result.contains("2"));
        assert!(result.contains("3"));
    }

    #[test]
    fn test_format_json_invalid() {
        let input = r#"{"key": invalid}"#;
        let result = format_json(input);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("parse error"));
    }

    #[test]
    fn test_format_json_empty() {
        let result = format_json("");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Empty"));
    }

    // ============ JSON Validation Tests ============

    #[test]
    fn test_validate_json_valid() {
        assert!(validate_json(r#"{"key":"value"}"#).is_ok());
        assert!(validate_json(r#"[1,2,3]"#).is_ok());
        assert!(validate_json(r#"null"#).is_ok());
    }

    #[test]
    fn test_validate_json_invalid() {
        let result = validate_json(r#"{"key": }"#);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_json_empty() {
        // Empty should be valid (no content)
        assert!(validate_json("").is_ok());
        assert!(validate_json("   ").is_ok());
    }

    // ============ XML Format Tests ============

    #[test]
    fn test_format_xml_simple() {
        let input = r#"<root><child>value</child></root>"#;
        let result = format_xml(input).unwrap();
        assert!(result.contains("<root>"));
        assert!(result.contains("<child>"));
        assert!(result.contains("value"));
    }

    #[test]
    fn test_format_xml_with_attributes() {
        let input = r#"<root attr="value"><child>text</child></root>"#;
        let result = format_xml(input).unwrap();
        assert!(result.contains("attr"));
        assert!(result.contains("<child>"));
    }

    #[test]
    fn test_format_xml_invalid() {
        let input = r#"<root><child>value</root>"#; // Mismatched tags
        let result = format_xml(input);
        assert!(result.is_err());
    }

    #[test]
    fn test_format_xml_empty() {
        let result = format_xml("");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Empty"));
    }

    // ============ XML Validation Tests ============

    #[test]
    fn test_validate_xml_valid() {
        assert!(validate_xml(r#"<root></root>"#).is_ok());
        assert!(validate_xml(r#"<root><child/></root>"#).is_ok());
    }

    #[test]
    fn test_validate_xml_invalid() {
        let result = validate_xml(r#"<root><child></root>"#);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_xml_empty() {
        // Empty should be valid
        assert!(validate_xml("").is_ok());
        assert!(validate_xml("   ").is_ok());
    }
}
