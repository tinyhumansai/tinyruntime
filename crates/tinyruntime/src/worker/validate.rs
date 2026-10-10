//! Preflight size checks before installation, serialization and native expansion.
use super::native::MAX_LINE;
use tinyruntime_bus::worker::{WorkerCommand, WorkerPlan, WorkerRequest};

const MAX_SCRIPT: usize = 1024 * 1024;

fn command(command: &WorkerCommand) -> Result<(), &'static str> {
    if !std::path::Path::new(&command.executable).is_absolute()
        || command.executable.len() > 4096
        || command.executable.contains('\0')
        || command.args.len() > 64
        || command.env.len() > 64
    {
        return Err("command_limit");
    }
    let bytes = command.args.iter().map(String::len).sum::<usize>()
        + command
            .env
            .iter()
            .map(|(k, v)| k.len() + v.len())
            .sum::<usize>();
    if bytes > 64 * 1024
        || command.args.iter().any(|s| s.contains('\0'))
        || command
            .env
            .iter()
            .any(|(k, v)| k.is_empty() || k.contains(['\0', '=']) || v.contains('\0'))
    {
        return Err("command_limit");
    }
    Ok(())
}

pub(super) fn plan(plan: &WorkerPlan) -> Result<(), &'static str> {
    if plan.source.len() > MAX_SCRIPT
        || plan.preparation.len() > 16
        || plan.backends.len() > 32
        || plan.backends.iter().any(|s| s.len() > 128)
        || plan.startup_timeout_ms == 0
        || plan.startup_timeout_ms > 30 * 60 * 1000
        || plan.request_timeout_ms == 0
        || plan.request_timeout_ms > 60 * 1000
        || plan.idle_timeout_ms > 24 * 60 * 60 * 1000
        || plan.idle_backend.as_ref().is_some_and(|s| s.len() > 128)
    {
        return Err("plan_limit");
    }
    command(&plan.command)?;
    for step in &plan.preparation {
        command(step)?;
    }
    Ok(())
}

/// Count serialized bytes through a bounded writer rather than allocating a clone.
struct Count(usize);
impl std::io::Write for Count {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0 = self.0.saturating_add(bytes.len());
        if self.0 >= MAX_LINE {
            return Err(std::io::ErrorKind::FileTooLarge.into());
        }
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

pub(super) fn request(request: &WorkerRequest) -> Result<(), &'static str> {
    if request.operation == 0 || request.handle.0.len() > 64 || request.method.len() > 256 {
        return Err("request_limit");
    }
    let mut count = Count(0);
    serde_json::to_writer(&mut count, &request.params).map_err(|_| "request_limit")?;
    if count.0 + request.method.len() + 128 >= MAX_LINE {
        return Err("request_limit");
    }
    Ok(())
}

#[cfg(test)]
#[path = "validate_tests.rs"]
mod tests;
