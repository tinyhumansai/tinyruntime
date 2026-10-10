//! Size, NUL and environment preflight boundaries before native allocation.
use super::*;

#[test]
fn invalid_command_vectors_and_plan_limits_are_rejected() {
    let mut command = WorkerCommand {
        executable: "/explicit-runtime".into(),
        args: vec!["x".repeat(65537)],
        env: Vec::new(),
    };
    assert_eq!(super::command(&command), Err("command_limit"));
    command.args.clear();
    command.env.push(("bad=name".into(), "value".into()));
    assert_eq!(super::command(&command), Err("command_limit"));
    command.env.clear();
    command.args.push("bad\0argument".into());
    assert_eq!(super::command(&command), Err("command_limit"));
    command.args.clear();
    let mut plan = WorkerPlan {
        source: "x".repeat(MAX_SCRIPT + 1),
        command,
        preparation: Vec::new(),
        backends: Vec::new(),
        startup_timeout_ms: 30000,
        request_timeout_ms: 60000,
        idle_backend: None,
        idle_timeout_ms: 0,
    };
    assert_eq!(super::plan(&plan), Err("plan_limit"));
    plan.source.clear();
    plan.preparation.push(WorkerCommand {
        executable: String::new(),
        args: Vec::new(),
        env: Vec::new(),
    });
    assert_eq!(super::plan(&plan), Err("command_limit"));
    plan.preparation.clear();
    plan.idle_backend = Some("x".repeat(129));
    assert_eq!(super::plan(&plan), Err("plan_limit"));
    let mut count = Count(0);
    assert!(std::io::Write::flush(&mut count).is_ok());
}
