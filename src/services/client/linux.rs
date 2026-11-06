use super::{Request, Scope, Service, run};
use crate::services::Action;
use std::{collections::BTreeMap, process::Command};

fn systemctl(scope: Scope, args: &[&str]) -> Command {
    let mut command = Command::new("systemctl");
    command.args(["--no-pager", "--no-ask-password"]);
    if scope == Scope::User {
        command.arg("--user");
    }
    command.args(args);
    command
}

pub fn inventory(scope: Scope) -> Result<Vec<Service>, String> {
    let files = run(systemctl(
        scope,
        &["list-unit-files", "--type=service", "--no-legend"],
    ))?;
    let units = run(systemctl(
        scope,
        &[
            "list-units",
            "--all",
            "--type=service",
            "--no-legend",
            "--plain",
            "--full",
        ],
    ))?;
    let mut services = parse(&files, &units);
    let details = run(systemctl(
        scope,
        &[
            "show",
            "--property=Id,MainPID,FragmentPath,CanStop",
            "--",
            "*.service",
        ],
    ))?;
    apply_details(&mut services, &details);
    Ok(services)
}

fn apply_details(services: &mut [Service], output: &str) {
    for block in output.split("\n\n") {
        let values: BTreeMap<_, _> = block
            .lines()
            .filter_map(|line| line.split_once('='))
            .collect();
        let Some(id) = values.get("Id") else {
            continue;
        };
        let Some(service) = services.iter_mut().find(|service| service.id == *id) else {
            continue;
        };
        service.pid = values
            .get("MainPID")
            .and_then(|pid| pid.parse::<u32>().ok())
            .filter(|pid| *pid > 0);
        if let Some(path) = values.get("FragmentPath")
            && !path.is_empty()
        {
            service.definition = (*path).into();
        }
        service.can_stop = values.get("CanStop") == Some(&"yes");
    }
}

fn parse(files: &str, units: &str) -> Vec<Service> {
    let mut services = BTreeMap::new();
    for line in files.lines() {
        let mut fields = line.split_whitespace();
        let (Some(id), Some(startup)) = (fields.next(), fields.next()) else {
            continue;
        };
        if !id.ends_with(".service") {
            continue;
        }
        services.insert(
            id.to_string(),
            Service {
                id: id.into(),
                name: id.trim_end_matches(".service").into(),
                description: String::new(),
                state: "Stopped".into(),
                startup: startup.into(),
                pid: None,
                definition: id.into(),
                can_stop: true,
                can_pause: false,
                loaded: false,
            },
        );
    }
    for line in units.lines() {
        let mut fields = line.split_whitespace();
        let (Some(id), Some(_load), Some(active), Some(sub)) =
            (fields.next(), fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        if !id.ends_with(".service") {
            continue;
        }
        let service = services.entry(id.into()).or_insert_with(|| Service {
            id: id.into(),
            name: id.trim_end_matches(".service").into(),
            description: String::new(),
            state: String::new(),
            startup: "transient".into(),
            pid: None,
            definition: id.into(),
            can_stop: true,
            can_pause: false,
            loaded: true,
        });
        service.loaded = true;
        service.state = match (active, sub) {
            ("active", "running") => "Running".into(),
            ("inactive", _) => "Stopped".into(),
            ("failed", _) => "Failed".into(),
            _ => format!("{active} / {sub}"),
        };
        service.description = fields.collect::<Vec<_>>().join(" ");
    }
    services.into_values().collect()
}

pub fn apply(request: &Request) -> Result<(), String> {
    run(action_command(request)?).map(|_| ())
}

fn action_command(request: &Request) -> Result<Command, String> {
    let verb = match request.action {
        Action::Start => "start",
        Action::Stop => "stop",
        Action::Restart => "restart",
        Action::Enable => "enable",
        Action::Disable => "disable",
        _ => return Err("systemd does not support this action.".into()),
    };
    // '--' prevents names from becoming options; reject wildcard expansion as well.
    if request.service.id.contains(['*', '?', '[', ']']) {
        return Err("Wildcard service targets are not supported.".into());
    }
    Ok(systemctl(request.scope, &[verb, "--", &request.service.id]))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn merges_installed_inactive_loaded_and_transient_services() {
        let services = parse(
            "fixture.service disabled enabled\nother.service static -\nworker@.service disabled enabled\n",
            "fixture.service loaded active running Test worker\nother.service loaded inactive dead Static worker\ntemp.service loaded failed failed Temporary worker\n",
        );
        assert_eq!(services.len(), 4);
        let fixture = services.iter().find(|s| s.id == "fixture.service").unwrap();
        assert_eq!(fixture.state, "Running");
        assert_eq!(fixture.startup, "disabled");
        assert_eq!(fixture.description, "Test worker");
        assert!(!fixture.allows(Action::Pause));
        assert_eq!(
            services
                .iter()
                .find(|s| s.id == "temp.service")
                .unwrap()
                .state,
            "Failed"
        );
        assert!(
            !services
                .iter()
                .find(|s| s.id == "worker@.service")
                .unwrap()
                .loaded
        );
    }

    #[test]
    fn reads_identity_and_stop_capability_without_property_order_assumptions() {
        let mut services = parse(
            "fixture.service enabled enabled",
            "fixture.service loaded active running Fixture",
        );
        apply_details(
            &mut services,
            "CanStop=no\nFragmentPath=/etc/systemd/system/fixture.service\nMainPID=123\nId=fixture.service\n",
        );
        assert_eq!(services[0].pid, Some(123));
        assert!(!services[0].allows(Action::Stop));
        assert_eq!(
            services[0].definition,
            "/etc/systemd/system/fixture.service"
        );
    }

    #[test]
    fn actions_keep_user_scope_and_reject_glob_targets() {
        let mut service = parse("fixture.service disabled enabled", "").remove(0);
        let mut request = Request {
            scope: Scope::User,
            service: service.clone(),
            action: Action::Start,
        };
        let command = action_command(&request).unwrap();
        let args: Vec<_> = command
            .get_args()
            .map(|arg| arg.to_str().unwrap())
            .collect();
        assert_eq!(
            args,
            [
                "--no-pager",
                "--no-ask-password",
                "--user",
                "start",
                "--",
                "fixture.service"
            ]
        );
        service.id = "*.service".into();
        request.service = service;
        assert!(action_command(&request).is_err());
        request.action = Action::Pause;
        assert!(action_command(&request).is_err());
    }
}
