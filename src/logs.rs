use serde_json::Value;
use std::{
    io::Read,
    process::{Command, Stdio},
    sync::OnceLock,
    time::{Duration, Instant},
};

pub const LIMIT: usize = 2000;
pub fn default_source() -> usize {
    if cfg!(target_os = "windows") || cfg!(target_os = "linux") {
        1
    } else {
        0
    }
}

#[derive(Debug)]
pub struct ReadError {
    pub message: String,
    pub permission_denied: bool,
}
impl ReadError {
    fn other(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            permission_denied: false,
        }
    }
}
impl From<String> for ReadError {
    fn from(message: String) -> Self {
        Self::other(message)
    }
}
impl From<&str> for ReadError {
    fn from(message: &str) -> Self {
        Self::other(message)
    }
}
impl std::fmt::Display for ReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

pub fn sources() -> &'static [&'static str] {
    if cfg!(target_os = "windows") {
        &["System", "Application", "Security"]
    } else if cfg!(target_os = "macos") {
        &["Unified log", "Kernel"]
    } else {
        &["System journal", "Current user", "Kernel"]
    }
}
#[derive(Debug)]
pub struct Entry {
    pub time: String,
    pub level: String,
    pub source: String,
    pub message: String,
    raw: String,
    pub search: String,
    details: OnceLock<String>,
}
impl Entry {
    #[cfg(any(test, feature = "ui-preview"))]
    pub(crate) fn test_entry(
        time: String,
        level: String,
        source: String,
        message: String,
        raw: String,
        search: String,
    ) -> Self {
        Self {
            time,
            level,
            source,
            message,
            raw,
            search,
            details: OnceLock::new(),
        }
    }

    pub fn details(&self) -> &str {
        self.details.get_or_init(|| {
            serde_json::from_str::<Value>(&self.raw)
                .ok()
                .and_then(|value| serde_json::to_string_pretty(&value).ok())
                .unwrap_or_else(|| self.raw.clone())
        })
    }
}
pub struct Snapshot {
    pub entries: Vec<Entry>,
    pub note: String,
    pub access_limited: bool,
}
fn text(v: &Value, key: &str) -> String {
    match &v[key] {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}
fn entry(v: Value, os: &str) -> Option<Entry> {
    let (time, level, source, message) = match os {
        "windows" => (
            text(&v, "Time"),
            match v["Level"].as_u64().unwrap_or(0) {
                1 => "Critical",
                2 => "Error",
                3 => "Warning",
                4 => "Info",
                5 => "Debug",
                _ => "Other",
            }
            .into(),
            format!("{} / {}", text(&v, "Provider"), text(&v, "Id")),
            text(&v, "Message"),
        ),
        "linux" => {
            let time = text(&v, "__REALTIME_TIMESTAMP")
                .parse::<i64>()
                .ok()
                .and_then(chrono::DateTime::from_timestamp_micros)
                .map(|d| d.with_timezone(&chrono::Local).to_rfc3339())
                .unwrap_or_default();
            let level = match text(&v, "PRIORITY").as_str() {
                "0" | "1" | "2" => "Critical",
                "3" => "Error",
                "4" => "Warning",
                "5" | "6" => "Info",
                "7" => "Debug",
                _ => "Other",
            };
            (
                time,
                level.into(),
                format!(
                    "{} {}",
                    text(&v, "SYSLOG_IDENTIFIER"),
                    text(&v, "_SYSTEMD_UNIT")
                ),
                text(&v, "MESSAGE"),
            )
        }
        _ => {
            if v.get("finished").is_some() {
                return None;
            }
            let level = match text(&v, "messageType").as_str() {
                "Fault" => "Critical",
                "Error" => "Error",
                "Info" | "Default" => "Info",
                "Debug" => "Debug",
                _ => "Other",
            };
            (
                text(&v, "timestamp"),
                level.into(),
                format!("{} {}", text(&v, "processImagePath"), text(&v, "subsystem")),
                text(&v, "eventMessage"),
            )
        }
    };
    // Keep one compact event copy for search and defer the larger pretty-printed
    // representation until the user opens or copies this event.
    let raw = serde_json::to_string(&v).unwrap_or_default();
    let search = format!("{time} {level} {source} {message} {raw}").to_lowercase();
    Some(Entry {
        time,
        level,
        source,
        message,
        raw,
        search,
        details: OnceLock::new(),
    })
}

// Drain both pipes concurrently, retaining bounded output. Never invoke a shell with user input.
struct Output {
    text: String,
    access_limited: bool,
}

fn permission_message(message: &str) -> bool {
    let message = message.to_lowercase();
    [
        "vijideck_log_access_denied",
        "access is denied",
        "permission denied",
        "unauthorized",
        "insufficient permissions",
        "insufficient privileges",
        "not permitted",
        "not authorized",
        "not seeing messages from other users",
        "must be root",
    ]
    .iter()
    .any(|needle| message.contains(needle))
}

fn command_failure(code: Option<i32>, stderr: &str, stdout: &str) -> ReadError {
    let detail = if stderr.trim().is_empty() {
        stdout.trim()
    } else {
        stderr.trim()
    };
    let permission_denied = permission_message(detail);
    let message = if permission_denied {
        "This log source needs administrator access. You can grant access or return to accessible logs.".to_string()
    } else if code == Some(126) || detail.contains("(-128)") {
        "Access request cancelled. Your existing log access is unchanged.".to_string()
    } else if detail.is_empty() {
        format!(
            "The log reader exited with status {} without an error description. Try refreshing or choose another time range.",
            code.map(|c| c.to_string())
                .unwrap_or_else(|| "unknown".into())
        )
    } else {
        format!("Cannot read logs: {detail}")
    };
    ReadError {
        message,
        permission_denied,
    }
}

fn output(mut command: Command, elevated: bool) -> Result<Output, ReadError> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command.spawn().map_err(|e| ReadError::other(
        format!("Cannot open the log reader: {e}. Check that the platform log utility and, if requesting access, the system authentication service are installed.")
    ))?;
    fn drain(mut pipe: impl Read + Send + 'static) -> std::thread::JoinHandle<Vec<u8>> {
        std::thread::spawn(move || {
            let mut result = Vec::new();
            let mut buf = [0; 8192];
            while let Ok(n) = pipe.read(&mut buf) {
                if n == 0 {
                    break;
                }
                let keep = n.min((32 * 1024 * 1024usize).saturating_sub(result.len()));
                result.extend_from_slice(&buf[..keep]);
            }
            result
        })
    }
    let stdout = drain(child.stdout.take().unwrap());
    let stderr = drain(child.stderr.take().unwrap());
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) if start.elapsed() < Duration::from_secs(if elevated { 120 } else { 30 }) => {
                std::thread::sleep(Duration::from_millis(50))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("Log query timed out. Choose a shorter time range and retry.".into());
            }
        }
    };
    let out = stdout.join().unwrap_or_default();
    let err = stderr.join().unwrap_or_default();
    if out.len() >= 32 * 1024 * 1024 {
        return Err("Log output exceeded 32 MiB. Choose a shorter time range.".into());
    }
    if !status.success() {
        return Err(command_failure(
            status.code(),
            &String::from_utf8_lossy(&err),
            &String::from_utf8_lossy(&out),
        ));
    }
    Ok(Output {
        text: String::from_utf8_lossy(&out).into_owned(),
        access_limited: permission_message(&String::from_utf8_lossy(&err)),
    })
}
pub fn load(
    source: usize,
    minutes: u32,
    range: Option<(String, String)>,
) -> Result<Snapshot, ReadError> {
    load_with_access(source, minutes, range, false)
}

pub fn load_with_access(
    source: usize,
    minutes: u32,
    range: Option<(String, String)>,
    elevated: bool,
) -> Result<Snapshot, ReadError> {
    let source = source.min(sources().len() - 1);
    let os = std::env::consts::OS;
    let mut command;
    if os == "windows" {
        command = Command::new("powershell.exe");
        let script = windows_query(sources()[source], minutes, range.as_ref());
        command.args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            &script,
        ]);
    } else if os == "linux" {
        command = if elevated {
            let mut cmd = Command::new("pkexec");
            cmd.arg("journalctl");
            cmd
        } else {
            Command::new("journalctl")
        };
        command.args(["--no-pager", "--output=json", "--reverse", "--lines=2000"]);
        if let Some((start, end)) = &range {
            command.args(["--since", start, "--until", end]);
        } else {
            command.args(["--since", &format!("-{minutes} minutes")]);
        }
        if source == 1 {
            command.arg("--user");
        } else if source == 2 {
            command.arg("--dmesg");
        }
    } else if os == "macos" {
        command = Command::new("/usr/bin/log");
        command.args(["show", "--style", "ndjson", "--info", "--debug"]);
        if let Some((start, end)) = &range {
            command.args(["--start", &mac_time(start), "--end", &mac_time(end)]);
        } else {
            command.args(["--last", &format!("{minutes}m")]);
        }
        if source == 1 {
            command.args(["--predicate", "process == 'kernel'"]);
        }
    } else {
        return Err("Native system logs are supported on Windows, macOS, and Linux.".into());
    }
    if os == "macos" && elevated {
        let shell = std::iter::once(command.get_program())
            .chain(command.get_args())
            .map(|part| shell_quote(&part.to_string_lossy()))
            .collect::<Vec<_>>()
            .join(" ");
        let script = format!(
            "do shell script {} with administrator privileges without altering line endings",
            serde_json::to_string(&shell).unwrap()
        );
        command = Command::new("/usr/bin/osascript");
        command.args(["-e", &script]);
    }
    let raw = output(command, elevated)?;
    let mut entries = Vec::new();
    for line in raw.text.lines().filter(|l| !l.trim().is_empty()) {
        let value =
            serde_json::from_str(line).map_err(|e| format!("Could not decode log output: {e}"))?;
        if let Some(e) = entry(value, os) {
            entries.push(e);
        }
    }
    if os == "macos" {
        entries.reverse();
    }
    entries.truncate(LIMIT);
    Ok(Snapshot {
        entries,
        access_limited: raw.access_limited,
        note: format!(
            "Newest {LIMIT} events at most. Search filters loaded events. Visibility depends on OS permissions; private fields may be redacted."
        ),
    })
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn windows_query(source: &str, minutes: u32, range: Option<&(String, String)>) -> String {
    format!(
        r#"[Console]::OutputEncoding=[System.Text.UTF8Encoding]::new(); $ErrorActionPreference='Stop'; try {{ $q=@{{LogName='{}';StartTime=(Get-Date).AddMinutes(-{})}}; if ('{}' -ne '') {{ $q.StartTime=[DateTimeOffset]::Parse('{}').LocalDateTime; $q.EndTime=[DateTimeOffset]::Parse('{}').LocalDateTime }}; Get-WinEvent -ListLog $q.LogName -ErrorAction Stop | Out-Null; Get-WinEvent -FilterHashtable $q -MaxEvents {} | ForEach-Object {{ @{{Time=$_.TimeCreated.ToString('o');Level=[int]$_.Level;Provider=$_.ProviderName;Id=$_.Id;Message=$_.Message;RecordId=$_.RecordId;Log=$_.LogName}} | ConvertTo-Json -Compress -Depth 4 }} }} catch {{ if ($_.FullyQualifiedErrorId -like 'NoMatchingEventsFound*') {{ exit 0 }}; $denied=$false; $exception=$_.Exception; while ($null -ne $exception) {{ if ($exception -is [System.UnauthorizedAccessException] -or $exception -is [System.Security.SecurityException] -or ($exception.HResult -band 65535) -eq 5) {{ $denied=$true }}; $exception=$exception.InnerException }}; if ($denied) {{ [Console]::Error.WriteLine('VIJIDECK_LOG_ACCESS_DENIED'); exit 5 }}; [Console]::Error.WriteLine($_.Exception.Message); exit 1 }}; exit 0"#,
        source,
        minutes,
        range.map(|r| r.0.as_str()).unwrap_or(""),
        range.map(|r| r.0.as_str()).unwrap_or(""),
        range.map(|r| r.1.as_str()).unwrap_or(""),
        LIMIT
    )
}

fn mac_time(value: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|d| d.format("%Y-%m-%d %H:%M:%S%:z").to_string())
        .unwrap_or_else(|_| value.to_string())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "reads the host system logs"]
    fn reads_host_logs() {
        let snapshot = load(default_source(), 1440, None).expect("read host logs");
        assert!(snapshot.entries.len() <= LIMIT);
        assert!(snapshot.entries.iter().all(|e| !e.details().is_empty()));
        println!("Read {} system events", snapshot.entries.len());
    }
    #[test]
    fn permission_errors_are_distinct_from_reader_failures_and_cancellation() {
        assert!(command_failure(Some(5), "VIJIDECK_LOG_ACCESS_DENIED", "").permission_denied);
        assert!(
            command_failure(
                Some(1),
                "No journal files were opened due to insufficient permissions.",
                ""
            )
            .permission_denied
        );
        let missing = command_failure(Some(1), "The log service is unavailable", "");
        assert!(!missing.permission_denied);
        assert!(missing.message.contains("log service"));
        let empty = command_failure(Some(1), "", "");
        assert!(!empty.permission_denied);
        assert!(empty.message.contains("without an error description"));
        assert!(
            command_failure(Some(126), "", "")
                .message
                .contains("cancelled")
        );
    }
    #[cfg(windows)]
    #[test]
    #[ignore = "queries Windows logs without requesting elevation"]
    fn empty_range_is_success_and_security_denial_is_identified() {
        let empty = load(
            1,
            60,
            Some((
                "2099-01-01T00:00:00+00:00".into(),
                "2099-01-01T01:00:00+00:00".into(),
            )),
        )
        .unwrap();
        assert!(empty.entries.is_empty());
        match load(2, 60, None) {
            Ok(_) => println!("Security logs are already readable for this account"),
            Err(error) => assert!(
                error.permission_denied,
                "unexpected Security log error: {error}"
            ),
        }
    }
    #[test]
    fn windows_message_and_level() {
        let e=entry(serde_json::json!({"Time":"today","Level":2,"Provider":"Service","Id":42,"Message":"Failed\nwith details"}),"windows").unwrap();
        assert_eq!(e.level, "Error");
        assert!(e.search.contains("failed\nwith details"));
        assert!(e.source.contains("42"));
    }
    #[test]
    fn journal_priority_and_time() {
        let e=entry(serde_json::json!({"__REALTIME_TIMESTAMP":"1000000","PRIORITY":"4","MESSAGE":"warning"}),"linux").unwrap();
        assert_eq!(e.level, "Warning");
        assert!(!e.time.is_empty());
    }
    #[test]
    fn mac_summary_is_not_event() {
        assert!(entry(serde_json::json!({"finished":1}), "macos").is_none());
    }
}
