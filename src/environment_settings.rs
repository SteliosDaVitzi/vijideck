//! Persistent environment settings, independent of rendering.
#[path = "environment_path.rs"]
mod path;
pub type Rows = Vec<(String, String, String)>;
static MUTATION: std::sync::Mutex<()> = std::sync::Mutex::new(());
pub fn save(name: &str, value: Option<&str>, scope: usize) -> Result<(), String> {
    let _guard = MUTATION
        .lock()
        .map_err(|_| "Settings worker stopped; restart VijiDeck before editing settings again.")?;
    save_inner(name, value, scope)
}
pub fn change_path(dir: &std::path::Path, add: bool) -> Result<String, String> {
    let _guard = MUTATION
        .lock()
        .map_err(|_| "Settings worker stopped; restart VijiDeck before editing settings again.")?;
    path::change(dir, add)
}
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .enumerate()
            .all(|(i, c)| c == '_' || c.is_ascii_alphabetic() || (i > 0 && c.is_ascii_digit()))
}

#[cfg(windows)]
fn powershell(script: &str) -> Result<String, String> {
    use std::os::windows::process::CommandExt;
    let out = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .creation_flags(0x08000000)
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_owned());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}
#[cfg(windows)]
pub fn persistent() -> Result<Vec<(String, String, String)>, String> {
    let out = read_powershell(
        r#"[Console]::OutputEncoding = [Text.Encoding]::UTF8; $r = @(); foreach ($s in @('User','Machine')) { $target = [EnvironmentVariableTarget]::$s; foreach ($e in [Environment]::GetEnvironmentVariables($target).GetEnumerator()) { $r += @{name=[string]$e.Key; value=[string]$e.Value; scope=if($s -eq 'Machine'){'System'}else{'User'}} } }; ConvertTo-Json -Compress -InputObject @($r)"#,
    )?;
    let data: serde_json::Value = serde_json::from_str(&out).map_err(|e| e.to_string())?;
    Ok(data
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| {
            Some((
                v["name"].as_str()?.into(),
                v["value"].as_str()?.into(),
                v["scope"].as_str()?.into(),
            ))
        })
        .collect())
}
#[cfg(windows)]
fn save_inner(name: &str, value: Option<&str>, scope: usize) -> Result<(), String> {
    if !valid_name(name) || value.is_some_and(|v| v.is_empty() || v.contains('\0')) {
        return Err(
            "Use a valid variable name and a nonempty value without NUL characters.".into(),
        );
    }
    // Base64 encodes UTF-16 PowerShell source, so values are never interpreted as commands.
    let quote = |s: &str| format!("'{}'", s.replace('\'', "''"));
    let expression = value.map(quote).unwrap_or_else(|| "$null".into());
    let script = format!(
        "try {{ [Environment]::SetEnvironmentVariable({}, {}, '{}'); exit 0 }} catch {{ exit 1 }}",
        quote(name),
        expression,
        if scope == 1 { "Machine" } else { "User" }
    );
    let bytes: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let encoded = base64(&bytes);
    if scope == 1 {
        powershell(&format!(
            "$p = Start-Process powershell.exe -Verb RunAs -Wait -PassThru -WindowStyle Hidden -ArgumentList '-NoProfile -NonInteractive -EncodedCommand {encoded}'; if ($p.ExitCode -ne 0) {{ throw 'The elevated change failed.' }}"
        ))?;
    } else {
        powershell(&format!(
            "& powershell.exe -NoProfile -NonInteractive -EncodedCommand {encoded}; if ($LASTEXITCODE -ne 0) {{ throw 'The change failed.' }}"
        ))?;
    }
    Ok(())
}
#[cfg(windows)]
fn base64(bytes: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::new();
    for c in bytes.chunks(3) {
        let n = ((c[0] as u32) << 16)
            | ((c.get(1).copied().unwrap_or(0) as u32) << 8)
            | c.get(2).copied().unwrap_or(0) as u32;
        s.push(T[((n >> 18) & 63) as usize] as char);
        s.push(T[((n >> 12) & 63) as usize] as char);
        s.push(if c.len() > 1 {
            T[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        s.push(if c.len() > 2 {
            T[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    s
}

#[cfg(not(windows))]
fn profile() -> Result<std::path::PathBuf, String> {
    Ok(
        std::path::PathBuf::from(std::env::var_os("HOME").ok_or("HOME is not set")?)
            .join(".profile"),
    )
}
#[cfg(not(windows))]
pub fn persistent() -> Result<Vec<(String, String, String)>, String> {
    let path = profile()?;
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e.to_string()),
    };
    Ok(content
        .lines()
        .filter_map(|line| {
            let (name, encoded) = line.strip_prefix("# VijiDeck variable ")?.split_once('=')?;
            let bytes: Option<Vec<u8>> = encoded
                .as_bytes()
                .chunks(2)
                .map(|c| {
                    if c.len() != 2 {
                        return None;
                    }
                    u8::from_str_radix(std::str::from_utf8(c).ok()?, 16).ok()
                })
                .collect();
            Some((name.into(), String::from_utf8(bytes?).ok()?, "User".into()))
        })
        .collect())
}
#[cfg(not(windows))]
fn save_inner(name: &str, value: Option<&str>, _: usize) -> Result<(), String> {
    save_profile(&profile()?, name, value)
}
#[cfg(not(windows))]
fn save_profile(path: &std::path::Path, name: &str, value: Option<&str>) -> Result<(), String> {
    if !valid_name(name) || value.is_some_and(|v| v.contains(['\0', '\n', '\r'])) {
        return Err("Use a valid shell variable name and a single-line value.".into());
    }
    crate::profile::update(path, |content| {
        let marker = format!("# VijiDeck variable {name}=");
        let export = format!("export {name}=");
        let mut lines = content.lines();
        let mut result = String::new();
        while let Some(line) = lines.next() {
            if line.starts_with(&marker) {
                if let Some(next) = lines.next()
                    && !next.starts_with(&export)
                {
                    result.push_str(next);
                    result.push('\n');
                }
            } else {
                result.push_str(line);
                result.push('\n');
            }
        }
        if let Some(v) = value {
            let hex: String = v.as_bytes().iter().map(|b| format!("{b:02x}")).collect();
            result.push_str(&format!(
                "{marker}{hex}\nexport {name}='{}'\n",
                v.replace('\'', "'\\''")
            ));
        }
        Ok(result)
    })
}

#[cfg(windows)]
fn read_powershell(script: &str) -> Result<String, String> {
    use std::os::windows::process::CommandExt;
    let mut command = std::process::Command::new("powershell.exe");
    command
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .creation_flags(0x08000000);
    let out = crate::command::capture(
        command,
        crate::command::Limits {
            timeout: std::time::Duration::from_secs(30),
            output_bytes: 8 * 1024 * 1024,
        },
    )
    .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_owned());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

#[cfg(all(test, not(windows)))]
mod tests {
    use super::*;
    #[test]
    fn variable_edits_preserve_unmanaged_text_and_path_entries() {
        let root =
            std::env::temp_dir().join(format!("vijideck-settings-test-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("profile");
        let original = "# login settings\nexport PATH='/tools':\"$PATH\" # VijiDeck managed PATH\n";
        std::fs::write(&path, original).unwrap();
        save_profile(&path, "EXAMPLE", Some("one' two")).unwrap();
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .starts_with(original)
        );
        save_profile(&path, "EXAMPLE", Some("updated")).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path)
                .unwrap()
                .matches("# VijiDeck variable EXAMPLE=")
                .count(),
            1
        );
        assert!(save_profile(&path, "BAD-NAME", Some("oops")).is_err());
        assert!(save_profile(&path, "EXAMPLE", Some("line\nbreak")).is_err());
        save_profile(&path, "EXAMPLE", None).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        std::fs::remove_dir_all(root).unwrap();
    }
}
