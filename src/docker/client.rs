//! Docker CLI transport shared by container and resource inventories.
use crate::command::{self, Limits};
use std::{process::Command, time::Duration};

pub(crate) fn docker(args: &[String]) -> Result<String, String> {
    let mut command = Command::new("docker");
    command.args(args);
    let output = command::capture(command, Limits {
        timeout: Duration::from_secs(60),
        output_bytes: 32 * 1024 * 1024,
    }).map_err(|error| match error {
        command::Error::Timeout => "Docker timed out after 60 seconds. Refresh to check the result.".into(),
        command::Error::OutputLimit => "Docker output exceeded 32 MiB. Narrow the inventory and refresh.".into(),
        other => format!("Cannot run Docker: {other}. Ensure Docker is installed and on PATH."),
    })?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(if detail.is_empty() {
            format!("Docker exited with {}. Refresh to check the result.", output.status)
        } else { detail });
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

pub(super) fn inventory() -> Result<Vec<super::model::Container>, String> {
    use super::model::{FORMAT, parse};
    docker(&["ps".into(), "--all".into(), "--no-trunc".into(), "--format".into(), FORMAT.into()])
        .and_then(|text| parse(&text))
}

pub(super) fn apply(request: &super::model::Request) -> Result<String, String> {
    use super::model::Action;
    let mut args = vec![match request.action {
        Action::Start => "start",
        Action::Stop => "stop",
        Action::Delete => "rm",
    }.to_owned()];
    if request.action == Action::Delete {
        args.push("--force".into());
    }
    args.extend(request.containers.iter().map(|container| container.id.clone()));
    docker(&args).map(|_| format!(
        "{} completed for {}. Refreshed container status.",
        request.action.label(), request.name
    ))
}
