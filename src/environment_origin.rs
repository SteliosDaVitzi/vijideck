use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Origin {
    Standalone(String),
    Managed(String),
    Bundled(String),
    Unknown,
}

impl Origin {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Standalone(_) => "Standalone",
            Self::Managed(_) => "Managed",
            Self::Bundled(_) => "Bundled",
            Self::Unknown => "Unknown origin",
        }
    }

    pub fn description(&self) -> String {
        match self {
            Self::Standalone(evidence) => format!("Standalone installation: {evidence}"),
            Self::Managed(evidence) => format!("Package or version manager: {evidence}"),
            Self::Bundled(evidence) => format!("Likely bundled with {evidence}"),
            Self::Unknown => {
                "Installation origin could not be determined from available evidence.".into()
            }
        }
    }
}

#[derive(Debug)]
struct InstalledApp {
    name: String,
    location: PathBuf,
}

#[derive(Default)]
pub struct OriginDetector {
    installed: Vec<InstalledApp>,
}

impl OriginDetector {
    pub fn new() -> Self {
        Self {
            installed: installed_apps(),
        }
    }

    pub fn classify(&self, tool: &str, path: &Path) -> Origin {
        // Normalize separators for deterministic matching, including Windows paths in tests.
        let text = path.to_string_lossy().replace('\\', "/").to_lowercase();
        let parts: Vec<_> = text.split('/').filter(|part| !part.is_empty()).collect();
        let contains = |name: &str| parts.contains(&name);
        for (component, name) in [
            (".rustup", "rustup"),
            (".cargo", "Cargo"),
            (".nvm", "nvm"),
            ("nvm", "nvm"),
            ("nvm-windows", "nvm-windows"),
            (".pyenv", "pyenv"),
            ("pyenv", "pyenv"),
            (".asdf", "asdf"),
            (".mise", "mise"),
            ("scoop", "Scoop"),
            ("chocolatey", "Chocolatey"),
            ("homebrew", "Homebrew"),
            ("cellar", "Homebrew"),
            ("miniconda3", "Conda"),
            ("anaconda3", "Conda"),
            ("conda", "Conda"),
            (".conda", "Conda"),
            (".sdkman", "SDKMAN!"),
            ("volta", "Volta"),
            (".volta", "Volta"),
            (".fnm", "fnm"),
        ] {
            if contains(component) {
                return Origin::Managed(format!("{name} directory"));
            }
        }
        if parts.iter().any(|part| {
            part.starts_with("blender ")
                && part[8..].chars().next().is_some_and(|c| c.is_ascii_digit())
        }) {
            return Origin::Bundled("Blender (application directory)".into());
        }
        for (component, name) in [
            ("blender", "Blender"),
            ("unity", "Unity"),
            ("unity hub", "Unity Hub"),
            ("android studio", "Android Studio"),
            ("visual studio", "Visual Studio"),
            ("jetbrains", "JetBrains software"),
            ("pycharm", "PyCharm"),
            ("intellij idea", "IntelliJ IDEA"),
            ("rider", "Rider"),
            ("gimp", "GIMP"),
            ("inkscape", "Inkscape"),
        ] {
            if contains(component) {
                return Origin::Bundled(format!("{name} (application directory)"));
            }
        }
        // An installer's directory is evidence only when it is a specific ancestor.
        let owner = self
            .installed
            .iter()
            .filter_map(|app| {
                let location = app
                    .location
                    .to_string_lossy()
                    .replace('\\', "/")
                    .trim_end_matches('/')
                    .to_lowercase();
                if location.split('/').filter(|p| !p.is_empty()).count() < 3 {
                    return None;
                }
                text.strip_prefix(&location)
                    .filter(|suffix| suffix.starts_with('/'))
                    .map(|_| (location.len(), app))
            })
            .max_by_key(|(length, _)| *length)
            .map(|(_, app)| app);
        if let Some(app) = owner {
            let name = app.name.to_lowercase();
            let standalone = match tool {
                "Python" | "Python 3" => {
                    name.starts_with("python ") || name.starts_with("python 3")
                }
                "Node.js" => name.starts_with("node.js"),
                ".NET" => name.starts_with("microsoft .net") || name.starts_with(".net "),
                "Java" | "Java compiler" => {
                    name.starts_with("java ") || name.contains("jdk") || name.contains("openjdk")
                }
                "Go" => name.starts_with("go programming language"),
                "Ruby" => name.starts_with("ruby "),
                "PHP" => name.starts_with("php "),
                "Git" => name == "git" || name.starts_with("git "),
                _ => false,
            };
            if standalone {
                return Origin::Standalone(format!("Windows installed app: {}", app.name));
            }
            return Origin::Bundled(format!("{} (Windows installed app location)", app.name));
        }
        Origin::Unknown
    }
}

#[cfg(not(windows))]
fn installed_apps() -> Vec<InstalledApp> {
    Vec::new()
}

#[cfg(windows)]
fn installed_apps() -> Vec<InstalledApp> {
    use std::{os::windows::process::CommandExt, process::Command};
    // Read-only registry enumeration, done once per scan and away from the UI thread.
    let script = r#"
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$roots = @('HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*', 'HKLM:\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\*', 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*')
$apps = @(Get-ItemProperty $roots -ErrorAction SilentlyContinue | Where-Object { $_.DisplayName -and $_.InstallLocation } | Select-Object DisplayName, InstallLocation)
$pythonRoots = @('HKCU:\Software\Python\PythonCore\*\InstallPath', 'HKLM:\Software\Python\PythonCore\*\InstallPath', 'HKLM:\Software\WOW6432Node\Python\PythonCore\*\InstallPath')
$pythons = @(Get-ItemProperty $pythonRoots -ErrorAction SilentlyContinue | Where-Object { $_.'(default)' } | ForEach-Object {
    [PSCustomObject]@{ DisplayName = 'Python (PythonCore registry registration)'; InstallLocation = $_.'(default)' }
})
$nodeRoots = @('HKCU:\Software\Node.js', 'HKLM:\Software\Node.js', 'HKLM:\Software\WOW6432Node\Node.js')
$nodes = @(Get-ItemProperty $nodeRoots -ErrorAction SilentlyContinue | Where-Object { $_.InstallPath } | ForEach-Object {
    [PSCustomObject]@{ DisplayName = 'Node.js (Node.js registry registration)'; InstallLocation = $_.InstallPath }
})
@($apps + $pythons + $nodes) | ConvertTo-Json -Compress
"#;
    let Ok(output) = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .creation_flags(0x08000000)
        .output()
    else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&output.stdout) else {
        return Vec::new();
    };
    let values = match value {
        serde_json::Value::Array(values) => values,
        serde_json::Value::Object(_) => vec![value],
        _ => return Vec::new(),
    };
    values
        .into_iter()
        .filter_map(|item| {
            let name = item.get("DisplayName")?.as_str()?.trim();
            let location = item
                .get("InstallLocation")?
                .as_str()?
                .trim()
                .trim_matches('"');
            if name.is_empty() || location.is_empty() {
                return None;
            }
            Some(InstalledApp {
                name: name.to_owned(),
                location: PathBuf::from(location),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uses_specific_evidence_and_does_not_guess_from_generic_paths() {
        let detector = OriginDetector {
            installed: vec![
                InstalledApp {
                    name: "Python 3.12 (64-bit)".into(),
                    location: PathBuf::from("C:/Users/Alice/Programs/Python312"),
                },
                InstalledApp {
                    name: "Example Editor".into(),
                    location: PathBuf::from("C:/Program Files/Example Editor"),
                },
                InstalledApp {
                    name: "Python (PythonCore registry registration)".into(),
                    location: PathBuf::from(
                        "C:/Users/steli/AppData/Local/Programs/Python/Python311/",
                    ),
                },
                InstalledApp {
                    name: "Node.js (Node.js registry registration)".into(),
                    location: PathBuf::from("C:/Program Files/nodejs/"),
                },
            ],
        };
        assert!(matches!(
            detector.classify(
                "Python",
                Path::new("C:/Users/Alice/Programs/Python312/python.exe")
            ),
            Origin::Standalone(_)
        ));
        assert!(matches!(
            detector.classify(
                "Python",
                Path::new(r"C:\Users\steli\AppData\Local\Programs\Python\Python311\python.exe")
            ),
            Origin::Standalone(_)
        ));
        assert!(matches!(
            detector.classify("Node.js", Path::new(r"C:\Program Files\nodejs\node.exe")),
            Origin::Standalone(_)
        ));
        assert!(matches!(
            detector.classify(
                "Node.js",
                Path::new(r"C:\Users\Alice\AppData\Roaming\nvm\v22\node.exe")
            ),
            Origin::Managed(_)
        ));
        assert!(matches!(
            detector.classify(
                "Python",
                Path::new("C:/Program Files/Example Editor/python/python.exe")
            ),
            Origin::Bundled(_)
        ));
        assert!(matches!(
            detector.classify(
                "Python",
                Path::new("C:/Program Files/Example Editor2/python.exe")
            ),
            Origin::Unknown
        ));
        assert!(matches!(
            detector.classify(
                "Python",
                Path::new("C:/Users/Alice/scoop/apps/python/current/python.exe")
            ),
            Origin::Managed(_)
        ));
        assert!(matches!(
            detector.classify(
                "Python",
                Path::new("C:/Program Files/Blender/4.0/python/bin/python.exe")
            ),
            Origin::Bundled(_)
        ));
        assert_eq!(
            detector.classify("Python", Path::new("C:/tools/python.exe")),
            Origin::Unknown
        );
    }
}
