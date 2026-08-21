//! Unit tests for the per-language settings payload.

use super::RuntimeSettings;

#[test]
fn new_enables_the_language_and_prefers_the_host_interpreter() {
    let settings = RuntimeSettings::new("v22.11.0");
    assert!(settings.enabled);
    assert!(settings.prefer_system);
    assert_eq!(settings.version, "v22.11.0");
}

#[test]
fn blank_optional_fields_read_as_absent() {
    let mut settings = RuntimeSettings::new("3.12");
    assert_eq!(settings.cache_dir(), None);
    assert_eq!(settings.release_tag(), None);
    assert_eq!(settings.preferred_command(), None);
    assert_eq!(settings.maximum_version(), None);

    settings.cache_dir = "   ".to_string();
    assert_eq!(settings.cache_dir(), None, "whitespace is still absent");
}

#[test]
fn set_optional_fields_are_trimmed() {
    let mut settings = RuntimeSettings::new("3.12");
    settings.preferred_command = "  /usr/bin/python3.12  ".to_string();
    assert_eq!(settings.preferred_command(), Some("/usr/bin/python3.12"));
}

#[test]
fn pins_its_wire_representation() {
    let value = serde_json::to_value(RuntimeSettings::new("3.12")).expect("settings serialise");
    assert_eq!(
        value,
        serde_json::json!({
            "enabled": true,
            "prefer_system": true,
            "version": "3.12",
            "maximum_version": "",
            "cache_dir": "",
            "release_tag": "",
            "preferred_command": "",
        })
    );
}
