//! Unit tests for the language identifier.

use super::{Language, NODEJS, PYTHON};

#[test]
fn normalises_case_and_surrounding_whitespace() {
    assert_eq!(Language::new("  NodeJS \n").as_str(), NODEJS);
    assert_eq!(Language::new("PYTHON").as_str(), PYTHON);
}

#[test]
fn constructors_match_the_published_identifiers() {
    assert_eq!(Language::nodejs().as_str(), NODEJS);
    assert_eq!(Language::python().as_str(), PYTHON);
}

#[test]
fn an_absent_language_is_empty_rather_than_defaulted() {
    assert!(Language::new("   ").is_empty());
    assert!(!Language::nodejs().is_empty());
}

#[test]
fn serialises_as_a_bare_string() {
    let encoded = serde_json::to_value(Language::python()).expect("language serialises");
    assert_eq!(encoded, serde_json::json!("python"));

    let decoded: Language = serde_json::from_value(serde_json::json!("NodeJS"))
        .expect("a language decodes from a bare string");
    assert_eq!(decoded, Language::nodejs());
}

#[test]
fn displays_as_its_identifier() {
    assert_eq!(Language::nodejs().to_string(), "nodejs");
}
