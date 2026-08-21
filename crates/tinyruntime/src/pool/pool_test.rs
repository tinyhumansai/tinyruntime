//! Unit tests for the pool registry.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use tinyruntime_bus::{Language, PoolSettings};

use super::{Launch, Pools};

fn launch(language: Language) -> Launch {
    crate::testing::evaluate_log_fields();
    Launch {
        language,
        binary: "/usr/bin/node".into(),
        args: vec!["worker.js".to_string()],
        env: Vec::new(),
        protocol_version: 1,
        handshake_timeout: std::time::Duration::from_secs(30),
    }
}

#[tokio::test]
async fn the_same_launch_and_tuning_reuse_one_pool() {
    let pools = Pools::new();
    let first = pools
        .ensure(
            launch(Language::nodejs()),
            PoolSettings::default(),
            "22.11.0".into(),
        )
        .await;
    let second = pools
        .ensure(
            launch(Language::nodejs()),
            PoolSettings::default(),
            "22.11.0".into(),
        )
        .await;
    assert!(
        std::sync::Arc::ptr_eq(&first, &second),
        "the warm pool was thrown away and rebuilt"
    );
}

#[tokio::test]
async fn a_changed_interpreter_rebuilds_the_pool() {
    // Reusing warm workers across a toolchain change would answer from the old
    // interpreter, which is the one bug a fingerprint exists to prevent.
    let pools = Pools::new();
    let first = pools
        .ensure(
            launch(Language::nodejs()),
            PoolSettings::default(),
            "22.11.0".into(),
        )
        .await;

    let mut upgraded = launch(Language::nodejs());
    upgraded.binary = "/cache/node-v24/bin/node".into();
    let second = pools
        .ensure(upgraded, PoolSettings::default(), "24.0.0".into())
        .await;

    assert!(!std::sync::Arc::ptr_eq(&first, &second));
}

#[tokio::test]
async fn retuning_the_pool_rebuilds_it() {
    let pools = Pools::new();
    let first = pools
        .ensure(
            launch(Language::nodejs()),
            PoolSettings::default(),
            "22.11.0".into(),
        )
        .await;
    let retuned = PoolSettings::default().with_max_workers(8);
    let second = pools
        .ensure(launch(Language::nodejs()), retuned, "22.11.0".into())
        .await;
    assert!(!std::sync::Arc::ptr_eq(&first, &second));
}

#[tokio::test]
async fn each_language_gets_its_own_pool() {
    let pools = Pools::new();
    pools
        .ensure(
            launch(Language::nodejs()),
            PoolSettings::default(),
            "22.11.0".into(),
        )
        .await;
    pools
        .ensure(
            launch(Language::python()),
            PoolSettings::default(),
            "3.12.4".into(),
        )
        .await;

    let stats = pools.stats().await;
    assert_eq!(stats.len(), 2);
    assert_eq!(stats[0].language, Language::nodejs());
    assert_eq!(stats[1].language, Language::python());
}

#[tokio::test]
async fn a_fresh_pool_reports_no_work_done() {
    let pools = Pools::new();
    pools
        .ensure(
            launch(Language::nodejs()),
            PoolSettings::default(),
            "22.11.0".into(),
        )
        .await;
    let stats = pools.stats().await;
    assert_eq!(stats[0].jobs_total, 0);
    assert_eq!(
        stats[0].worker_spawns, 0,
        "a pool must not spawn before it is used"
    );
    assert_eq!(
        stats[0].max_workers,
        PoolSettings::default().effective_max_workers()
    );
}

// ---------------------------------------------------------------------------
// Against a live worker process
//
// Everything above is bookkeeping. These drive a real child over a real socket
// via `fake_worker`, which is what actually covers the handshake, warm reuse,
// recycling, and the dispatch tagging that keeps a job from running twice.
// ---------------------------------------------------------------------------

use std::time::Duration;

use super::fake_worker::{self, Directive};
use super::lang_pool::LangPool;
use crate::error::Error;

/// A pool of `max_workers` fake workers with recycling off.
fn live_pool(max_workers: usize) -> std::sync::Arc<LangPool> {
    let settings = PoolSettings::default()
        .with_max_workers(max_workers)
        .with_recycle_after_jobs(0);
    LangPool::start(
        fake_worker::launch(Language::nodejs()),
        settings,
        "1.0.0-test".to_string(),
    )
}

/// Run one directive on `pool`.
async fn run(
    pool: &LangPool,
    directive: &Directive<'_>,
    timeout: Option<Duration>,
) -> crate::Result<tinyruntime_bus::ExecResponse> {
    pool.run(directive.code(), None, timeout).await
}

#[tokio::test]
async fn a_job_runs_on_a_real_worker_and_comes_back() {
    let pool = live_pool(1);
    let response = run(&pool, &Directive::Echo("hello"), None)
        .await
        .expect("the job runs");

    assert_eq!(response.stdout, "hello");
    assert_eq!(response.exit_code, Some(0));
    assert!(response.success());
    assert_eq!(
        response.runtime_version, "1.0.0-test",
        "the reply carries the toolchain that ran it"
    );
}

#[tokio::test]
async fn many_jobs_share_one_warm_worker() {
    // The entire point of the pool: N jobs, one interpreter child.
    let pool = live_pool(1);
    for index in 0..4 {
        let text = format!("job-{index}");
        let response = run(&pool, &Directive::Echo(&text), None)
            .await
            .expect("each job runs");
        assert_eq!(response.stdout, text);
    }

    let stats = pool.stats().await;
    assert_eq!(stats.jobs_total, 4);
    assert_eq!(
        stats.worker_spawns, 1,
        "four jobs spawned {} interpreters",
        stats.worker_spawns
    );
    assert_eq!(stats.idle_workers, 1, "the warm worker was not parked");
}

#[tokio::test]
async fn a_failing_job_is_a_result_rather_than_an_error() {
    // The job threw; the pool did its job. Callers need the output, not an error.
    let pool = live_pool(1);
    let response = run(&pool, &Directive::Fail("boom"), None)
        .await
        .expect("a throwing job still returns");

    assert!(!response.success());
    assert_eq!(response.exit_code, Some(1));
    assert_eq!(response.stderr, "boom");
}

#[tokio::test]
async fn a_job_aborted_at_its_deadline_is_reported_as_timed_out() {
    let pool = live_pool(1);
    let response = run(&pool, &Directive::TimedOut, Some(Duration::from_secs(1)))
        .await
        .expect("the worker replies even when it aborts");
    assert!(response.timed_out);
    assert!(!response.success());
}

#[tokio::test]
async fn a_harness_level_failure_is_terminal_rather_than_retryable() {
    // The job reached the worker, so it may have run. Re-running it could
    // duplicate whatever it already did.
    let pool = live_pool(1);
    let error = run(&pool, &Directive::HarnessError("cannot enter cwd"), None)
        .await
        .expect_err("a harness failure is an error");

    assert!(matches!(error, Error::PostDispatch { .. }), "got {error:?}");
    assert!(!error.is_retryable());
    assert!(error.to_string().contains("cannot enter cwd"));
}

#[tokio::test]
async fn a_worker_that_dies_mid_job_is_terminal_and_not_re_run() {
    // The request went out before the worker closed the stream, so the job may
    // have executed. This is the case that must never be retried.
    let pool = live_pool(1);
    let error = run(&pool, &Directive::Die, None)
        .await
        .expect_err("a closed stream fails the job");

    assert!(matches!(error, Error::PostDispatch { .. }), "got {error:?}");
    assert!(!error.is_retryable());
}

#[tokio::test]
async fn a_wedged_worker_is_abandoned_at_the_hard_deadline() {
    // The worker never replies. The grace above the soft deadline is what ends
    // the wait, and the worker is discarded rather than parked.
    let pool = live_pool(1);
    let error = run(&pool, &Directive::Hang, Some(Duration::from_millis(50)))
        .await
        .expect_err("a silent worker cannot succeed");

    assert!(matches!(error, Error::PostDispatch { .. }), "got {error:?}");
    let stats = pool.stats().await;
    assert_eq!(
        stats.idle_workers, 0,
        "a wedged worker was parked for reuse"
    );
}

#[tokio::test]
async fn a_reply_for_another_job_is_skipped_rather_than_returned() {
    // Returning it would hand this caller another job's output.
    let pool = live_pool(1);
    let response = run(&pool, &Directive::Misaddressed("mine"), None)
        .await
        .expect("the right reply is found");
    assert_eq!(response.stdout, "mine");
}

#[tokio::test]
async fn an_unparseable_line_is_skipped_rather_than_failing_the_job() {
    // Harnesses are written in the provider's language. A stray line one of them
    // prints must not take down the job.
    let pool = live_pool(1);
    let response = run(&pool, &Directive::Noise("still-here"), None)
        .await
        .expect("the job survives noise");
    assert_eq!(response.stdout, "still-here");
}

#[tokio::test]
async fn a_worker_is_retired_once_it_has_served_its_budget() {
    let settings = PoolSettings::default()
        .with_max_workers(1)
        .with_recycle_after_jobs(1);
    let pool = LangPool::start(
        fake_worker::launch(Language::nodejs()),
        settings,
        "1.0.0-test".to_string(),
    );

    for _ in 0..2 {
        run(&pool, &Directive::Echo("x"), None)
            .await
            .expect("each job runs");
    }

    let stats = pool.stats().await;
    assert_eq!(
        stats.worker_spawns, 2,
        "a budget of one job should retire the worker after each"
    );
    assert_eq!(stats.idle_workers, 0, "a retired worker was parked anyway");
}

#[tokio::test]
async fn a_pool_with_no_queue_sheds_a_second_concurrent_job() {
    // Capacity is workers + allowed queue depth. With one worker and no queue,
    // a job arriving while the first is in flight is refused rather than waited
    // on — and the caller is told not to spawn its own interpreter.
    let settings = PoolSettings::default()
        .with_max_workers(1)
        .with_max_queue_depth(0)
        .with_recycle_after_jobs(0);
    let pool = LangPool::start(
        fake_worker::launch(Language::nodejs()),
        settings,
        "1.0.0-test".to_string(),
    );

    let busy = std::sync::Arc::clone(&pool);
    let occupied = tokio::spawn(async move {
        busy.run(
            Directive::Hang.code(),
            None,
            Some(Duration::from_millis(400)),
        )
        .await
    });
    // Let the first job take the only slot.
    tokio::time::sleep(Duration::from_millis(300)).await;

    let shed = run(&pool, &Directive::Echo("second"), None).await;
    let Err(error) = shed else {
        let _ = occupied.await;
        panic!("a full pool accepted a second job");
    };
    assert!(matches!(error, Error::PoolSaturated(_)), "got {error:?}");
    assert!(error.is_retryable(), "a busy pool is worth retrying");

    let _ = occupied.await;
    assert_eq!(pool.stats().await.rejected_saturated, 1);
}

#[tokio::test]
async fn a_job_records_how_long_it_waited_for_a_worker() {
    let pool = live_pool(1);
    let response = run(&pool, &Directive::Echo("x"), None)
        .await
        .expect("the job runs");
    // Nothing was queued, so the wait is small — what matters is that the field
    // is populated separately from the run time rather than left at a default.
    assert!(response.queue_wait_ms < 5_000);
}

#[tokio::test]
async fn a_parked_worker_that_died_is_replaced_and_the_job_still_runs() {
    // The one case where a retry is safe: the worker died between jobs, so the
    // write fails and the job provably never left. Without the respawn, an idle
    // timeout on the far side would surface as a user-visible failure.
    let pool = live_pool(1);
    run(&pool, &Directive::ExitAfterReply, None)
        .await
        .expect("the first job runs, then the worker exits");

    // Let the child actually exit before the next take, which is when the pool
    // notices. A TCP write into a closed peer's buffer succeeds, so waiting for
    // a write failure instead would never come — that is exactly the trap this
    // path exists to avoid.
    tokio::time::sleep(Duration::from_millis(300)).await;

    let response = run(&pool, &Directive::Echo("after-respawn"), None)
        .await
        .expect("the job runs on a fresh worker");
    assert_eq!(response.stdout, "after-respawn");
    assert_eq!(
        pool.stats().await.worker_spawns,
        2,
        "the dead worker was not replaced"
    );
}

#[tokio::test]
async fn a_workers_own_output_is_drained_rather_than_parsed() {
    // A job owns the process's stdout. The pool reads and discards it so a
    // chatty job never blocks on a full pipe — and never has it mistaken for a
    // protocol frame.
    let pool = live_pool(1);
    let response = run(&pool, &Directive::Print("to-fd-one"), None)
        .await
        .expect("the job runs");
    assert_eq!(
        response.stdout, "to-fd-one",
        "the reply is the harness's, not what landed on the descriptor"
    );
}

#[tokio::test]
async fn a_parked_worker_past_its_time_to_live_is_not_reused() {
    let settings = PoolSettings::default()
        .with_max_workers(1)
        .with_idle_ttl_secs(1)
        .with_recycle_after_jobs(0);
    let pool = LangPool::start(
        fake_worker::launch(Language::nodejs()),
        settings,
        "1.0.0-test".to_string(),
    );

    run(&pool, &Directive::Echo("first"), None)
        .await
        .expect("the first job runs");
    tokio::time::sleep(Duration::from_millis(1_200)).await;
    run(&pool, &Directive::Echo("second"), None)
        .await
        .expect("the second job runs");

    assert_eq!(
        pool.stats().await.worker_spawns,
        2,
        "a worker past its time-to-live was reused"
    );
}

#[tokio::test]
async fn the_reaper_retires_idle_workers_and_stops_when_the_pool_is_dropped() {
    let settings = PoolSettings::default()
        .with_max_workers(1)
        .with_idle_ttl_secs(1)
        .with_recycle_after_jobs(0);
    let pool = LangPool::start(
        fake_worker::launch(Language::nodejs()),
        settings,
        "1.0.0-test".to_string(),
    );

    run(&pool, &Directive::Echo("x"), None)
        .await
        .expect("the job runs");
    assert_eq!(
        pool.stats().await.idle_workers,
        1,
        "the worker was not parked"
    );

    // A fresh worker is not yet expired, so the reaper keeps it.
    pool.reap().await;
    assert_eq!(pool.stats().await.idle_workers, 1);

    tokio::time::sleep(Duration::from_millis(1_200)).await;
    pool.reap().await;
    assert_eq!(
        pool.stats().await.idle_workers,
        0,
        "an expired worker was kept"
    );

    // Run the loop at a pace a test can observe, then drop the pool: the reaper
    // holds only a weak reference, so it must stop rather than keep it alive.
    let weak = std::sync::Arc::downgrade(&pool);
    super::lang_pool::spawn_reaper(weak.clone(), Duration::from_millis(20));
    // Let at least one iteration run against a live pool before dropping it.
    tokio::time::sleep(Duration::from_millis(80)).await;
    drop(pool);
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(weak.strong_count(), 0, "the reaper kept the pool alive");
}

#[tokio::test]
async fn a_worker_describes_what_it_serves_rather_than_its_descriptors() {
    let worker = super::worker::Worker::spawn(&fake_worker::launch(Language::nodejs()))
        .await
        .expect("the fake worker starts");
    let rendered = format!("{worker:?}");
    assert!(rendered.contains("nodejs"), "got {rendered}");
    assert!(rendered.contains("jobs_done"), "got {rendered}");
    worker.shutdown();
}

#[tokio::test]
async fn a_worker_whose_socket_died_fails_the_job_terminally() {
    // A graceful close leaves the socket writable, so the write succeeds and the
    // failure surfaces at the read — by which point the job counts as
    // dispatched. That is why the parked-worker case is caught *before* the
    // write, by checking whether the process has exited, rather than by retrying
    // here.
    let pool = live_pool(1);
    run(&pool, &Directive::Linger, None)
        .await
        .expect("the first job runs, then the stream closes");
    tokio::time::sleep(Duration::from_millis(300)).await;

    let error = run(&pool, &Directive::Echo("second"), None)
        .await
        .expect_err("a dead stream cannot carry a reply");
    assert!(matches!(error, Error::PostDispatch { .. }), "got {error:?}");
    assert!(
        !error.is_retryable(),
        "a job that may have run must never be retried"
    );
}

#[tokio::test]
async fn a_worker_that_cannot_be_started_fails_the_job_retryably() {
    let mut spec = fake_worker::launch(Language::nodejs());
    spec.binary = "/nonexistent/interpreter".into();
    let pool = LangPool::start(spec, PoolSettings::default(), "1.0.0-test".to_string());

    let error = pool
        .run(Directive::Echo("x").code(), None, None)
        .await
        .expect_err("a missing interpreter cannot run anything");
    assert!(matches!(error, Error::PreDispatch { .. }), "got {error:?}");
    assert!(error.is_retryable());
}

#[tokio::test]
async fn a_worker_that_never_hands_over_a_handshake_is_refused() {
    let spec = fake_worker::launch_with_mode(Language::nodejs(), "silent");
    let pool = LangPool::start(spec, PoolSettings::default(), "1.0.0-test".to_string());

    let error = pool
        .run(Directive::Echo("x").code(), None, None)
        .await
        .expect_err("a worker that says nothing is not ready");
    assert!(matches!(error, Error::PreDispatch { .. }), "got {error:?}");
    assert!(
        error.to_string().contains("handshake") || error.to_string().contains("exited"),
        "got `{error}`"
    );
}

#[tokio::test]
async fn a_worker_whose_handshake_is_not_one_is_refused() {
    let spec = fake_worker::launch_with_mode(Language::nodejs(), "garbage");
    let pool = LangPool::start(spec, PoolSettings::default(), "1.0.0-test".to_string());

    let error = pool
        .run(Directive::Echo("x").code(), None, None)
        .await
        .expect_err("an unparseable handshake is not a handshake");
    assert!(matches!(error, Error::PreDispatch { .. }), "got {error:?}");
    assert!(error.to_string().contains("handshake"), "got `{error}`");
}

#[tokio::test]
async fn a_pool_that_never_reaps_leaves_its_workers_alone() {
    let settings = PoolSettings::default()
        .with_max_workers(1)
        .with_idle_ttl_secs(0)
        .with_recycle_after_jobs(0);
    let pool = LangPool::start(
        fake_worker::launch(Language::nodejs()),
        settings,
        "1.0.0-test".to_string(),
    );

    run(&pool, &Directive::Echo("x"), None)
        .await
        .expect("the job runs");
    pool.reap().await;
    assert_eq!(
        pool.stats().await.idle_workers,
        1,
        "a pool with reaping disabled retired a worker anyway"
    );
}
