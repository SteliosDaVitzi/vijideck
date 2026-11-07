use super::{Request, Scope, Service};
use std::{process::Command, time::Duration};

#[cfg(any(target_os = "linux", test))]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod linux;
#[cfg(any(target_os = "macos", test))]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
mod macos;
#[cfg(any(windows, test))]
#[cfg_attr(not(windows), allow(dead_code))]
mod windows;

pub fn run(mut command: Command) -> Result<String, String> {
    command.env("LC_ALL", "C").env("SYSTEMD_COLORS", "0");
    let output = crate::command::capture(command, crate::command::Limits {
        timeout: Duration::from_secs(45), output_bytes: 8 * 1024 * 1024,
    }).map_err(|error| format!("Service command failed: {error}. Refresh before retrying; a timed-out action may have completed."))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let detail = if stderr.trim().is_empty() {
            stdout.trim()
        } else {
            stderr.trim()
        };
        return Err(format!(
            "Service manager returned {}: {}. System changes may require administrator/root privileges; protected services can reject changes.",
            output.status,
            detail
                .lines()
                .next()
                .unwrap_or("No error details supplied")
                .chars()
                .take(600)
                .collect::<String>()
        ));
    }
    String::from_utf8(output.stdout).map_err(|_| "Service manager returned invalid UTF-8.".into())
}

pub fn inventory(scope: Scope) -> Result<Vec<Service>, String> {
    #[cfg(windows)]
    let result = windows::inventory(scope);
    #[cfg(target_os = "linux")]
    let result = linux::inventory(scope);
    #[cfg(target_os = "macos")]
    let result = macos::inventory(scope);
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    let result: Result<Vec<Service>, String> = {
        let _ = scope;
        Err("This platform has no supported service manager.".into())
    };
    let mut services = result?;
    services.sort_by_cached_key(|service| service.name.to_lowercase());
    Ok(services)
}

pub fn apply(request: &Request) -> Result<String, String> {
    if request.service.manager != super::Manager::native() {
        return Err("This service belongs to a different platform service manager.".into());
    }
    // Re-read identity, configuration and runtime state immediately before dispatch.
    super::validate(request, &inventory(request.scope)?)?;
    #[cfg(windows)]
    windows::apply(request)?;
    #[cfg(target_os = "linux")]
    linux::apply(request)?;
    #[cfg(target_os = "macos")]
    macos::apply(request)?;
    Ok(format!(
        "{}: {} completed. Refresh shows the latest service state.",
        request.service.name,
        request.action.label()
    ))
}
