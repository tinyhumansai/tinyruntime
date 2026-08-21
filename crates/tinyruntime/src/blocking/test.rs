//! Unit tests for the blocking-pool wrapper.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use tinyruntime_bus::Language;

use super::run;
use crate::error::Error;

#[tokio::test]
async fn a_value_comes_back_from_the_blocking_pool() {
    let value = run(&Language::nodejs(), || Ok(41 + 1))
        .await
        .expect("the work completes");
    assert_eq!(value, 42);
}

#[tokio::test]
async fn a_failure_from_the_work_is_returned_unchanged() {
    // The wrapper must not reclassify what the work already decided.
    let error = run(&Language::nodejs(), || {
        Err::<(), _>(Error::EmptyInstall(Language::nodejs()))
    })
    .await
    .expect_err("the work failed");
    assert!(matches!(error, Error::EmptyInstall(_)), "got {error:?}");
}

#[tokio::test]
async fn a_panicking_task_is_reported_rather_than_propagated() {
    // The only way to reach the join-failure arm, and the reason this wrapper
    // exists in one place instead of once per call site. A panic in a module
    // loaded into someone's process should surface as an error, not unwind
    // through whichever task happened to be awaiting it.
    let error = run(&Language::python(), || -> crate::Result<()> {
        panic!("the blocking work panicked")
    })
    .await
    .expect_err("a panicking task cannot succeed");

    let Error::Install { language, reason } = &error else {
        panic!("got {error:?}");
    };
    assert_eq!(language, &Language::python());
    assert!(reason.contains("did not finish"), "got `{reason}`");
}
