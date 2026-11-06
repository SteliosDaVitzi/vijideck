use super::{Request, Scope, Service, run};
use crate::services::Action;
use serde_json::Value;
use std::{collections::BTreeMap, process::Command};

// Foundation reads XML and binary property lists without executing their contents.
const PLISTS: &str = r#"
ObjC.import('Foundation');
function run(argv) {
    const home = ObjC.unwrap($.NSHomeDirectory());
    const dirs = argv[0] === 'system'
        ? ['/System/Library/LaunchDaemons', '/Library/LaunchDaemons']
        : ['/System/Library/LaunchAgents', '/Library/LaunchAgents', home + '/Library/LaunchAgents'];
    const rows = [];
    dirs.forEach(function(dir) {
        const files = $.NSFileManager.defaultManager.contentsOfDirectoryAtPathError(dir, null);
        if (!files) return;
        const names = ObjC.deepUnwrap(files) || [];
        names.forEach(function(name) {
            if (!name.endsWith('.plist')) return;
            const path = dir + '/' + name;
            const dict = $.NSDictionary.dictionaryWithContentsOfFile(path);
            if (!dict) return;
            const plist = ObjC.deepUnwrap(dict);
            if (!plist || typeof plist.Label !== 'string') return;
            rows.push({label: plist.Label, path: path, disabled: plist.Disabled === true,
                program: plist.Program || (plist.ProgramArguments || []).join(' ')});
        });
    });
    return JSON.stringify(rows);
}
"#;

fn launchctl(args: &[&str]) -> Command {
    let mut command = Command::new("/bin/launchctl");
    command.args(args);
    command
}

fn domain(scope: Scope) -> Result<String, String> {
    if scope == Scope::System {
        return Ok("system".into());
    }
    let mut command = Command::new("/usr/bin/id");
    command.arg("-u");
    let uid: u32 = run(command)?
        .trim()
        .parse()
        .map_err(|_| "Could not determine the current user's launchd domain.")?;
    Ok(format!("gui/{uid}"))
}

pub fn inventory(scope: Scope) -> Result<Vec<Service>, String> {
    let domain = domain(scope)?;
    let mut command = Command::new("/usr/bin/osascript");
    command.args([
        "-l",
        "JavaScript",
        "-e",
        PLISTS,
        if scope == Scope::System {
            "system"
        } else {
            "user"
        },
    ]);
    let plists = run(command)?;
    let loaded = run(launchctl(&["print", &domain]))?;
    let disabled = run(launchctl(&["print-disabled", &domain]))?;
    parse(&plists, &loaded, &disabled)
}

fn parse(plists: &str, domain: &str, overrides: &str) -> Result<Vec<Service>, String> {
    let rows: Vec<Value> =
        serde_json::from_str(plists).map_err(|e| format!("Invalid launchd inventory: {e}"))?;
    let mut services = BTreeMap::new();
    for row in rows {
        let Some(id) = row["label"].as_str() else {
            continue;
        };
        services.insert(
            id.to_string(),
            Service {
                id: id.into(),
                name: id.into(),
                description: row["program"].as_str().unwrap_or_default().into(),
                state: "Not loaded".into(),
                startup: if row["disabled"].as_bool().unwrap_or(false) {
                    "Disabled"
                } else {
                    "Enabled"
                }
                .into(),
                pid: None,
                definition: row["path"].as_str().unwrap_or_default().into(),
                can_stop: true,
                can_pause: false,
                loaded: false,
            },
        );
    }
    // launchctl's diagnostic format can change. Only accept rows in its services
    // table; never infer a running service from endpoint or environment entries.
    let mut in_services = false;
    let mut recognized_table = false;
    for line in domain.lines() {
        let line = line.trim();
        if line == "services = {" {
            in_services = true;
            recognized_table = true;
            continue;
        }
        if in_services && line == "}" {
            in_services = false;
        }
        if !in_services {
            continue;
        }
        if line.is_empty()
            || line
                .to_ascii_lowercase()
                .split_whitespace()
                .eq(["pid", "status", "label"])
        {
            continue;
        }
        let fields: Vec<_> = line.split_whitespace().collect();
        if fields.len() != 3 {
            return Err(
                "Unsupported launchctl service table; refresh after checking your macOS version."
                    .into(),
            );
        }
        let pid = if fields[0] == "-" || fields[0] == "0" {
            None
        } else {
            fields[0].parse::<u32>().ok()
        };
        if pid.is_none() && !matches!(fields[0], "-" | "0") {
            return Err(
                "Unsupported launchctl process identity; service actions remain unavailable."
                    .into(),
            );
        }
        let id = fields[2];
        let service = services.entry(id.into()).or_insert_with(|| Service {
            id: id.into(),
            name: id.into(),
            description: "Loaded launchd job; no standard plist found".into(),
            state: String::new(),
            startup: "Unknown".into(),
            pid: None,
            definition: String::new(),
            can_stop: true,
            can_pause: false,
            loaded: true,
        });
        service.loaded = true;
        service.pid = pid;
        service.state = if pid.is_some() { "Running" } else { "Stopped" }.into();
    }
    if !recognized_table {
        return Err(
            "This macOS launchctl inventory format is unsupported; no actions were enabled.".into(),
        );
    }
    for line in overrides.lines() {
        let Some((label, disabled)) = line.trim().split_once(" => ") else {
            continue;
        };
        let Some(service) = services.get_mut(label.trim_matches('"')) else {
            continue;
        };
        match disabled.trim() {
            "true" => service.startup = "Disabled".into(),
            "false" => service.startup = "Enabled".into(),
            _ => {}
        }
    }
    Ok(services.into_values().collect())
}

pub fn apply(request: &Request) -> Result<(), String> {
    let domain = domain(request.scope)?;
    let target = format!("{domain}/{}", request.service.id);
    match request.action {
        Action::Start => {
            if !request.service.loaded {
                if request.service.definition.is_empty() {
                    return Err("No configured property list was found for this job.".into());
                }
                run(launchctl(&[
                    "bootstrap",
                    &domain,
                    &request.service.definition,
                ]))?;
            }
            run(launchctl(&["kickstart", &target]))?;
        }
        Action::Stop => {
            run(launchctl(&["bootout", &target]))?;
        }
        Action::Restart => {
            run(launchctl(&["kickstart", "-k", &target]))?;
        }
        Action::Enable => {
            run(launchctl(&["enable", &target]))?;
        }
        Action::Disable => {
            run(launchctl(&["disable", &target]))?;
        }
        _ => return Err("launchd does not support this action.".into()),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn merges_configured_loaded_and_disabled_jobs_without_endpoints() {
        let services = parse(r#"[{"label":"com.fixture","path":"/Library/LaunchDaemons/com.fixture.plist","disabled":false}]"#,
            "system = {\n services = {\n  123 0 com.fixture\n  0 (pe) com.other\n }\n endpoints = {\n  999 0 com.endpoint\n }\n}",
            "disabled services = {\n \"com.fixture\" => true\n}").unwrap();
        assert_eq!(services.len(), 2);
        assert_eq!(services[0].pid, Some(123));
        assert_eq!(services[0].startup, "Disabled");
        assert!(services[0].allows(Action::Stop));
        assert!(!services[0].allows(Action::Pause));
        assert!(parse("[]", "unexpected format", "").is_err());
        assert!(parse("[]", "services = {\n com.fixture = {\n", "").is_err());
    }
}
