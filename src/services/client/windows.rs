use super::{Request, Scope, Service, run};
use crate::services::Action;
use serde_json::Value;
use std::process::Command;

fn powershell(script: &str) -> Command {
    let mut command = Command::new("powershell.exe");
    command.args([
        "-NoLogo",
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        script,
    ]);
    command
}

pub fn inventory(scope: Scope) -> Result<Vec<Service>, String> {
    if scope != Scope::System {
        return Err("Windows services use the system service manager.".into());
    }
    parse(&run(powershell(
        r#"
$ErrorActionPreference='Stop'
[Console]::OutputEncoding=[System.Text.UTF8Encoding]::new($false)
$details = @{}
try { Get-CimInstance Win32_Service -OperationTimeoutSec 8 | ForEach-Object { $details[$_.Name] = $_ } } catch { }
$rows = @(Get-Service | ForEach-Object {
    $service = $_
    $detail = $details[$service.Name]
    $config = $null
    if ($null -eq $detail) {
        $config = Get-ItemProperty -LiteralPath ('Registry::HKEY_LOCAL_MACHINE\SYSTEM\CurrentControlSet\Services\' + $service.Name) -ErrorAction SilentlyContinue
    }
    $stop = $false; $pause = $false; $mode = 'Unknown'
    try { $stop = $service.CanStop; $pause = $service.CanPauseAndContinue; $mode = $service.StartType.ToString() } catch { }
    [PSCustomObject]@{
        Name = $service.Name; DisplayName = $service.DisplayName
        State = $service.Status.ToString(); StartMode = $mode
        AcceptStop = $stop; AcceptPause = $pause
        ProcessId = if ($null -ne $detail) { $detail.ProcessId } else { $null }
        PathName = if ($null -ne $detail) { $detail.PathName } else { $config.ImagePath }
        Description = if ($null -ne $detail) { $detail.Description } else { $config.Description }
    }
})
ConvertTo-Json -InputObject $rows -Compress -Depth 3
"#,
    ))?)
}

fn parse(output: &str) -> Result<Vec<Service>, String> {
    let value: Value = serde_json::from_str(output.trim_start_matches('\u{feff}'))
        .map_err(|e| format!("Invalid Windows service inventory: {e}"))?;
    let rows = match &value {
        Value::Array(rows) => rows.clone(),
        Value::Object(_) => vec![value],
        Value::Null => Vec::new(),
        _ => return Err("Unexpected Windows service inventory.".into()),
    };
    rows.iter()
        .map(|row| {
            let text = |key: &str| row[key].as_str().unwrap_or_default().to_string();
            let id = text("Name");
            if id.is_empty() {
                return Err("Service inventory contains an empty name.".into());
            }
            Ok(Service {
                id,
                name: text("DisplayName"),
                description: text("Description"),
                state: text("State"),
                startup: match text("StartMode").as_str() {
                    "Auto" => "Automatic".into(),
                    mode => mode.into(),
                },
                pid: row["ProcessId"]
                    .as_u64()
                    .and_then(|pid| u32::try_from(pid).ok())
                    .filter(|pid| *pid > 0),
                definition: text("PathName"),
                can_stop: row["AcceptStop"].as_bool().unwrap_or(false),
                can_pause: row["AcceptPause"].as_bool().unwrap_or(false),
                loaded: true,
            })
        })
        .collect()
}

pub fn apply(request: &Request) -> Result<(), String> {
    run(action_command(request)).map(|_| ())
}

fn action_command(request: &Request) -> Command {
    let action = match request.action {
        Action::Start => "Start-Service -InputObject $service",
        Action::Stop => "Stop-Service -InputObject $service",
        Action::Restart => "Restart-Service -InputObject $service",
        Action::Pause => "Suspend-Service -InputObject $service",
        Action::Resume => "Resume-Service -InputObject $service",
        Action::Enable => "Set-Service -InputObject $service -StartupType Automatic",
        Action::Disable => "Set-Service -InputObject $service -StartupType Disabled",
        Action::Manual => "Set-Service -InputObject $service -StartupType Manual",
    };
    // Never interpolate service names into PowerShell source or use wildcard lookup.
    let mut command = powershell(&format!(
        "$ErrorActionPreference='Stop'; $service = Get-Service | Where-Object {{ $_.Name -eq $env:VIJIDECK_SERVICE_NAME }}; if (@($service).Count -ne 1) {{ throw 'Service identity changed' }}; {action}"
    ));
    command.env("VIJIDECK_SERVICE_NAME", &request.service.id);
    command
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn handles_single_empty_and_multiple_services() {
        let row = r#"{"Name":"fixture'[*]","DisplayName":"Fixture","State":"Running","StartMode":"Auto","ProcessId":123,"AcceptStop":true,"AcceptPause":false}"#;
        let services = parse(row).unwrap();
        assert_eq!(services[0].id, "fixture'[*]");
        assert_eq!(services[0].startup, "Automatic");
        assert!(services[0].can_stop);
        assert!(!services[0].can_pause);
        assert_eq!(parse(&format!("[{row},{row}]")).unwrap().len(), 2);
        assert!(parse("null").unwrap().is_empty());
        assert!(parse("{}").is_err());
    }

    #[test]
    fn passes_service_identity_as_data_without_shell_or_wildcard_interpolation() {
        let service = parse(r#"{"Name":"fixture'[*]; Write-Output INJECTED","State":"Stopped","StartMode":"Manual"}"#).unwrap().remove(0);
        let command = action_command(&Request {
            scope: Scope::System,
            service: service.clone(),
            action: Action::Start,
        });
        let script = command.get_args().last().unwrap().to_string_lossy();
        assert!(!script.contains(&service.id));
        assert!(!script.contains("-Force"));
        assert!(script.contains("-InputObject"));
        assert!(
            command
                .get_envs()
                .any(|(key, value)| key == "VIJIDECK_SERVICE_NAME"
                    && value == Some(std::ffi::OsStr::new(&service.id)))
        );
    }
}
