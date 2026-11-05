//! Container inventory and pure Compose grouping rules. No UI or CLI dependencies.
use std::collections::{BTreeMap, HashSet};

pub(super) const FORMAT: &str = r#"{"id":{{json .ID}},"name":{{json .Names}},"image":{{json .Image}},"state":{{json .State}},"status":{{json .Status}},"ports":{{json .Ports}},"project":{{json (.Label "com.docker.compose.project")}},"service":{{json (.Label "com.docker.compose.service")}}}"#;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Container {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) image: String,
    pub(super) state: String,
    pub(super) status: String,
    pub(super) ports: String,
    pub(super) project: String,
    pub(super) service: String,
}

impl Container {
    pub(super) fn active(&self) -> bool {
        matches!(self.state.as_str(), "running" | "restarting" | "paused")
    }

    pub(super) fn startable(&self) -> bool {
        matches!(self.state.as_str(), "created" | "exited")
    }

    fn matches(&self, query: &[&str]) -> bool {
        let text = format!(
            "{} {} {} {} {} {} {}",
            self.id, self.name, self.image, self.status, self.ports, self.project, self.service
        ).to_lowercase();
        query.iter().all(|word| text.contains(word))
    }
}

pub(super) fn parse(text: &str) -> Result<Vec<Container>, String> {
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let value: serde_json::Value = serde_json::from_str(line)
                .map_err(|error| format!("Invalid Docker response: {error}"))?;
            let get = |key: &str| {
                value[key].as_str().map(str::to_owned)
                    .ok_or_else(|| format!("Docker response is missing {key}"))
            };
            let id = get("id")?;
            if id.len() != 64 || !id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return Err("Docker returned an invalid container ID".into());
            }
            Ok(Container {
                id,
                name: get("name")?,
                image: get("image")?,
                state: get("state")?,
                status: get("status")?,
                ports: get("ports")?,
                project: get("project")?,
                service: get("service")?,
            })
        })
        .collect()
}

pub(super) struct DisplayRow {
    pub(super) key: String,
    pub(super) name: String,
    pub(super) project: bool,
    pub(super) child: bool,
    pub(super) running: usize,
    pub(super) containers: Vec<Container>,
}

pub(super) fn display_rows(
    containers: &[Container],
    expanded: &HashSet<String>,
    query: &str,
    running_only: bool,
) -> Vec<DisplayRow> {
    let mut groups: BTreeMap<String, Vec<Container>> = BTreeMap::new();
    let mut standalone = Vec::new();
    for container in containers {
        if container.project.is_empty() {
            standalone.push(container.clone());
        } else {
            groups.entry(container.project.clone()).or_default().push(container.clone());
        }
    }
    let query = query.to_lowercase();
    let terms: Vec<_> = query.split_whitespace().collect();
    let visible = |container: &Container| {
        (!running_only || container.state == "running") && container.matches(&terms)
    };
    let container_row = |container: &Container, child| DisplayRow {
        key: container.id.clone(),
        name: container.name.clone(),
        project: false,
        child,
        running: usize::from(container.state == "running"),
        containers: vec![container.clone()],
    };
    let mut rows = Vec::new();
    for (project, mut members) in groups {
        members.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
        if !members.iter().any(&visible) {
            continue;
        }
        rows.push(DisplayRow {
            key: format!("compose:{project}"),
            name: project.clone(),
            project: true,
            child: false,
            running: members.iter().filter(|container| container.state == "running").count(),
            containers: members.clone(),
        });
        if expanded.contains(&project) {
            rows.extend(members.iter().filter(|container| visible(container))
                .map(|container| container_row(container, true)));
        }
    }
    standalone.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
    rows.extend(standalone.iter().filter(|container| visible(container))
        .map(|container| container_row(container, false)));
    rows
}

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Action {
    Start,
    Stop,
    Delete,
}

impl Action {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Start => "Start",
            Self::Stop => "Stop",
            Self::Delete => "Delete",
        }
    }
}

#[derive(Clone)]
pub(super) struct Request {
    pub(super) action: Action,
    pub(super) name: String,
    pub(super) containers: Vec<Container>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_compose_and_standalone_and_searches() {
        let row = |project: &str| serde_json::json!({"id": "a".repeat(64), "name":"web-1", "image":"nginx", "state":"running", "status":"Up 2 hours (healthy)", "ports":"0.0.0.0:8080->80/tcp", "project":project, "service":"web"}).to_string();
        let containers = parse(&format!("{}\n{}\n", row("shop"), row(""))).unwrap();
        assert_eq!(containers.len(), 2);
        assert_eq!(containers[0].project, "shop");
        assert!(containers[1].project.is_empty());
        assert!(containers[0].matches(&["shop", "8080", "healthy"]));
        assert!(!containers[1].matches(&["shop"]));
        assert!(containers[0].active());
        assert!(!containers[0].startable());
    }

    #[test]
    fn groups_are_stable_and_actions_keep_hidden_members() {
        let make = |id: char, name: &str, state: &str, project: &str| Container {
            id: id.to_string().repeat(64), name: name.into(), image: "image".into(), state: state.into(),
            status: state.into(), ports: String::new(), project: project.into(), service: String::new(),
        };
        let mut inventory = vec![make('a', "web", "running", "shop"), make('b', "db", "exited", "shop"), make('c', "cache", "running", "")];
        let expanded = HashSet::from(["shop".into()]);
        let rows = display_rows(&inventory, &expanded, "", false);
        assert_eq!(rows.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["shop", "db", "web", "cache"]);
        assert!(rows[1].child);
        let filtered = display_rows(&inventory, &expanded, "web", true);
        assert_eq!(filtered.len(), 2);
        assert_eq!(filtered[0].containers.len(), 2, "group actions must include filtered children");
        inventory.reverse();
        assert_eq!(display_rows(&inventory, &expanded, "", false).iter().map(|r| &r.key).collect::<Vec<_>>(), rows.iter().map(|r| &r.key).collect::<Vec<_>>());
    }

    #[test]
    fn rejects_malformed_inventory() {
        assert!(parse("not json").is_err());
        assert!(parse(r#"{"id":"--help"}"#).is_err());
        assert!(parse("").unwrap().is_empty());
    }
}
