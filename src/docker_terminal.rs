use std::process::Command;
#[cfg(not(windows))]
use std::process::Stdio;

pub fn open(id: &str) -> Result<String, String> {
    // Only full Docker IDs enter terminal command strings on macOS and Windows.
    if id.len() != 64 || !id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("Cannot open terminal: invalid container ID.".into());
    }
    launch(id)?;
    Ok("Opened a container shell in your desktop terminal.".into())
}

fn detach(mut command: Command) -> std::io::Result<()> {
    // A newly allocated Windows console needs its own interactive standard handles.
    #[cfg(not(windows))]
    command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    let mut child = command.spawn()?;
    std::thread::spawn(move || { child.wait().ok(); });
    Ok(())
}

#[cfg(target_os = "linux")]
fn terminal_candidates(id: &str, desktop: &str) -> Vec<(&'static str, Vec<String>)> {
    let mut terminals = if desktop.to_lowercase().contains("kde") {
        vec![("konsole", "-e"), ("x-terminal-emulator", "-e"), ("gnome-terminal", "--")]
    } else {
        vec![("x-terminal-emulator", "-e"), ("gnome-terminal", "--"), ("konsole", "-e")]
    };
    terminals.extend([("xfce4-terminal", "-x"), ("kitty", "--"), ("alacritty", "-e"), ("xterm", "-e")]);
    terminals.into_iter().map(|(exe, separator)| {
        (exe, [separator, "docker", "exec", "-it", id, "sh"].into_iter().map(str::to_owned).collect())
    }).collect()
}

#[cfg(target_os = "linux")]
fn launch(id: &str) -> Result<(), String> {
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    for (exe, args) in terminal_candidates(id, &desktop) {
        let mut command = Command::new(exe);
        command.args(args);
        match detach(command) {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(format!("Cannot open {exe}: {error}")),
        }
    }
    Err("No supported desktop terminal found. Install Konsole, GNOME Terminal, Xfce Terminal, Kitty, Alacritty or xterm.".into())
}

#[cfg(windows)]
fn launch(id: &str) -> Result<(), String> {
    let mut terminal = Command::new("wt.exe");
    terminal.args(["docker", "exec", "-it", id, "sh"]);
    match detach(terminal) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            use std::os::windows::process::CommandExt;
            let mut terminal = Command::new("powershell.exe");
            terminal.args(["-NoExit", "-Command", &format!("& docker exec -it {id} sh")]);
            terminal.creation_flags(0x00000010); // CREATE_NEW_CONSOLE
            detach(terminal).map_err(|error| format!("Cannot open PowerShell: {error}"))
        }
        Err(error) => Err(format!("Cannot open Windows Terminal: {error}")),
    }
}

#[cfg(target_os = "macos")]
fn launch(id: &str) -> Result<(), String> {
    let mut terminal = Command::new("osascript");
    terminal.args(["-e", &format!("tell application \"Terminal\"\nactivate\ndo script \"docker exec -it {id} sh\"\nend tell")]);
    detach(terminal).map_err(|error| format!("Cannot open Terminal: {error}"))
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn launch(_: &str) -> Result<(), String> {
    Err("Opening a container terminal is unsupported on this platform.".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_command_text_in_place_of_container_id() {
        for id in ["--help", "container; touch /tmp/unsafe", "", &"z".repeat(64)] {
            assert!(open(id).unwrap_err().contains("invalid container ID"));
        }
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn terminal_arguments_keep_docker_and_container_separate() {
        let id = "a".repeat(64);
        for desktop in ["KDE", "GNOME"] {
            let candidates = terminal_candidates(&id, desktop);
            assert_eq!(candidates[0].0, if desktop == "KDE" { "konsole" } else { "x-terminal-emulator" });
            for (_, args) in candidates {
                assert_eq!(&args[1..], ["docker", "exec", "-it", &id, "sh"]);
            }
        }
    }
}
