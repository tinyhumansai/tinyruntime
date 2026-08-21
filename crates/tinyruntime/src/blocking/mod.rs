//! Running the synchronous parts of an install on a blocking thread.
//!
//! Unpacking an archive, renaming a directory into place, and taking a file
//! lock are all synchronous and can take a while. Running them inline would
//! stall the async runtime this module shares with the bus, so each goes to the
//! blocking pool.
//!
//! That leaves every call site with the same join-failure arm to write. Doing it
//! here once means the arm exists in one place — and, unlike a copy per call
//! site, it can actually be tested: a task that panics is the only way to reach
//! it, and that is easy to arrange deliberately and impossible to arrange by
//! accident.

use tinyruntime_bus::Language;

use crate::error::{Error, Result};

/// Run `work` on the blocking pool.
///
/// # Errors
///
/// Whatever `work` returns, or [`Error::Install`] when the task itself did not
/// finish — which means it panicked, since nothing here cancels it.
pub(crate) async fn run<T>(
    language: &Language,
    work: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T>
where
    T: Send + 'static,
{
    match tokio::task::spawn_blocking(work).await {
        Ok(result) => result,
        Err(error) => Err(Error::Install {
            language: language.clone(),
            reason: format!("the task did not finish: {error}"),
        }),
    }
}

#[cfg(test)]
mod test;
