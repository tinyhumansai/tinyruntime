use super::{EvaluateFields, evaluate_log_fields};

#[test]
fn the_subscriber_answers_every_call_a_span_would_make() {
    // `tracing` only reaches these when something opens a span. Nothing in
    // this crate does yet, so they are exercised directly — an unimplemented
    // arm here would take down the first code that ever adds one.
    use tracing::Subscriber as _;

    evaluate_log_fields();
    let subscriber = EvaluateFields;
    let id = tracing::span::Id::from_u64(1);

    subscriber.enter(&id);
    subscriber.record_follows_from(&id, &id);
    subscriber.exit(&id);
}
