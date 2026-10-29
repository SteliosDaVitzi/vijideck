use std::path::Path;

#[cfg(windows)]
pub fn change(dir: &Path, add: bool) -> Result<String, String> {
    use std::os::windows::process::CommandExt;
    let script = r#"
$ErrorActionPreference = 'Stop'
$dir = $env:VIJIDECK_PATH_DIR
$key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Environment', $true)
if ($null -eq $key) { throw 'Cannot open the user environment registry key.' }
$current = $key.GetValue('Path', '', [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
$kind = if ($key.GetValue('Path') -eq $null) { [Microsoft.Win32.RegistryValueKind]::ExpandString } else { $key.GetValueKind('Path') }
$parts = @($current -split ';' | Where-Object { $_ -ne '' })
$matches = @($parts | Where-Object { $_.TrimEnd('\') -ieq $dir.TrimEnd('\') })
if ($env:VIJIDECK_PATH_ADD -eq '1') {
  if ($matches.Count -gt 0) { throw 'This directory is already in the user PATH.' }
  $parts += $dir
} else {
  if ($matches.Count -eq 0) { throw 'This directory is not in the user PATH. It may come from the system PATH or the process environment.' }
  $parts = @($parts | Where-Object { $_.TrimEnd('\') -ine $dir.TrimEnd('\') })
}
$key.SetValue('Path', ($parts -join ';'), $kind)
$key.Dispose()
"#;
    let output = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .env("VIJIDECK_PATH_DIR", dir)
        .env("VIJIDECK_PATH_ADD", if add { "1" } else { "0" })
        .creation_flags(0x08000000)
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
    }
    Ok("User PATH updated. Restart applications to pick up the change; VijiDeck's current PATH will not change.".into())
}

#[cfg(not(windows))]
pub fn change(dir: &Path, add: bool) -> Result<String, String> {
    use std::fs;
    let home = std::env::var_os("HOME").ok_or("HOME is not set")?;
    let profile = std::path::PathBuf::from(home).join(".profile");
    let path = dir.to_str().ok_or("This path is not valid UTF-8")?;
    if path.contains(['\n', '\r', ':']) {
        return Err("This directory cannot be represented in PATH safely.".into());
    }
    let quoted = format!("'{}'", path.replace('\'', "'\\''"));
    let marker = format!("export PATH={quoted}:\"$PATH\" # VijiDeck managed PATH");
    let content = match fs::read_to_string(&profile) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e.to_string()),
    };
    let present = content.lines().any(|line| line == marker);
    if add && present {
        return Err("This directory is already managed in ~/.profile.".into());
    }
    if !add && !present {
        return Err("This directory is not managed by VijiDeck in ~/.profile. Remove it from its original configuration instead.".into());
    }
    let updated = if add {
        format!(
            "{}{}{}\n",
            content,
            if content.is_empty() || content.ends_with('\n') {
                ""
            } else {
                "\n"
            },
            marker
        )
    } else {
        content
            .split_inclusive('\n')
            .filter(|line| line.trim_end_matches(['\r', '\n']) != marker)
            .collect::<String>()
    };
    fs::write(&profile, updated).map_err(|e| e.to_string())?;
    Ok("Updated ~/.profile. New login shells that read this file will use the change; graphical apps and other shells may use different configuration.".into())
}
