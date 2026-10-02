//! Unit tests for the crate-wide error type.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::Error;
use tinyruntime_bus::Language;

#[test]
fn messages_are_lowercase_and_unpunctuated() {
    let errors = [
        Error::UnknownLanguage(Language::nodejs()),
        Error::LanguageDisabled(Language::python()),
        Error::LanguageMissing,
        Error::NotProvisioned(Language::nodejs()),
        Error::PoolSaturated(Language::python()),
        Error::EmptyInstall(Language::python()),
        Error::DigestMismatch {
            language: Language::nodejs(),
        },
        Error::Storage("cache root is not writable".to_string()),
    ];
    for error in errors {
        let rendered = error.to_string();
        assert!(
            !rendered.ends_with('.'),
            "`{rendered}` ends with punctuation"
        );
        let first = rendered.chars().next().expect("a non-empty message");
        assert!(
            !first.is_uppercase(),
            "`{rendered}` starts with a capital letter"
        );
    }
}

#[test]
fn a_digest_mismatch_is_not_retryable() {
    // Retrying a transfer that succeeded and produced the wrong bytes just
    // produces the wrong bytes again, and every retry runs the verification that
    // already refused them.
    assert!(
        !Error::DigestMismatch {
            language: Language::nodejs(),
        }
        .is_retryable()
    );
    assert!(
        Error::Download {
            language: Language::nodejs(),
            reason: "connection reset".to_string(),
        }
        .is_retryable()
    );
}

#[test]
fn a_post_dispatch_failure_is_terminal_but_a_pre_dispatch_one_is_not() {
    // This is the distinction that keeps a job from running twice: only a job
    // that provably never reached a worker may be retried.
    assert!(
        Error::PreDispatch {
            language: Language::nodejs(),
            reason: "worker died on write".to_string(),
        }
        .is_retryable()
    );
    assert!(
        !Error::PostDispatch {
            language: Language::nodejs(),
            reason: "worker closed its protocol stream".to_string(),
        }
        .is_retryable()
    );
}

#[test]
fn a_saturated_pool_is_retryable_and_an_unknown_language_is_not() {
    assert!(Error::PoolSaturated(Language::nodejs()).is_retryable());
    assert!(!Error::UnknownLanguage(Language::nodejs()).is_retryable());
}

#[test]
fn a_contract_mismatch_names_the_version_it_refused() {
    let rendered = Error::ProviderContract {
        language: Language::python(),
        major: 2,
        minor: 3,
    }
    .to_string();
    assert!(rendered.contains("2.3"), "got `{rendered}`");
    assert!(rendered.contains("python"), "got `{rendered}`");
}
