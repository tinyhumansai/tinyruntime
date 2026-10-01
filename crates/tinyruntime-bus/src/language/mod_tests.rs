//! Unit tests for the language identifier.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

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

#[test]
fn decoding_normalises_the_same_way_the_constructor_does() {
    // Not a restatement of the constructor test: a derived transparent decode
    // would skip normalisation entirely and route a valid request nowhere.
    let decoded: Language =
        serde_json::from_value(serde_json::json!("  PYTHON  ")).expect("language decodes");
    assert_eq!(decoded, Language::python());
    assert_eq!(decoded.as_str(), "python");
}

#[test]
fn a_language_can_be_built_from_either_string_type() {
    assert_eq!(Language::from("NodeJS"), Language::nodejs());
    assert_eq!(Language::from(String::from(" PYTHON ")), Language::python());
}

#[test]
fn ordering_is_by_identifier_so_a_listing_is_stable() {
    let mut languages = vec![Language::python(), Language::nodejs()];
    languages.sort();
    assert_eq!(languages, vec![Language::nodejs(), Language::python()]);
}

#[test]
fn the_identifier_constants_are_what_the_constructors_produce() {
    assert_eq!(NODEJS, "nodejs");
    assert_eq!(PYTHON, "python");
    assert_eq!(Language::new(NODEJS), Language::nodejs());
}

#[test]
fn an_empty_language_displays_as_nothing_rather_than_a_placeholder() {
    assert_eq!(Language::new("").to_string(), "");
}
