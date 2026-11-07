//! Service identity, capabilities and validation are independent of the view.
pub(crate) mod client;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Scope {
    #[default]
    System,
    User,
}

impl Scope {
    pub fn label(self) -> &'static str {
        match self {
            Self::System => "System",
            Self::User => "Current user",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Manager {
    Systemd,
    Launchd,
    Windows,
}
impl Manager {
    pub fn supports(self, action: Action) -> bool {
        self == Self::Windows || !matches!(action, Action::Pause | Action::Resume | Action::Manual)
    }
    pub fn native() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::Launchd
        } else {
            Self::Systemd
        }
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RuntimeState {
    Running,
    Stopped,
    Paused,
    Failed,
    Active,
    Other,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum StartupMode {
    Enabled,
    Disabled,
    Manual,
    Masked,
    Other,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Service {
    pub manager: Manager,
    pub id: String,
    pub name: String,
    pub description: String,
    pub state: String,
    pub startup: String,
    pub pid: Option<u32>,
    pub definition: String,
    pub can_stop: bool,
    pub can_pause: bool,
    pub loaded: bool,
}

impl Service {
    pub fn runtime(&self) -> RuntimeState {
        match self.state.as_str() {
            "Running" => RuntimeState::Running,
            "Stopped" | "Not loaded" => RuntimeState::Stopped,
            "Paused" => RuntimeState::Paused,
            "Failed" => RuntimeState::Failed,
            state if state.starts_with("active /") => RuntimeState::Active,
            _ => RuntimeState::Other,
        }
    }
    pub fn startup_mode(&self) -> StartupMode {
        match self.startup.as_str() {
            "Automatic" | "Enabled" | "enabled" | "enabled-runtime" => StartupMode::Enabled,
            "Disabled" | "disabled" => StartupMode::Disabled,
            "Manual" => StartupMode::Manual,
            "masked" | "masked-runtime" => StartupMode::Masked,
            _ => StartupMode::Other,
        }
    }
    pub fn running(&self) -> bool {
        self.runtime() == RuntimeState::Running
    }
    pub fn allows(&self, action: Action) -> bool {
        if !self.manager.supports(action) {
            return false;
        }
        let state = self.runtime();
        let startup = self.startup_mode();
        match action {
            Action::Start => {
                matches!(state, RuntimeState::Stopped | RuntimeState::Failed)
                    && startup != StartupMode::Masked
                    && (startup != StartupMode::Disabled || self.manager == Manager::Systemd)
                    && !self.id.contains("@.service")
            }
            Action::Stop => {
                self.can_stop
                    && matches!(
                        state,
                        RuntimeState::Running | RuntimeState::Paused | RuntimeState::Active
                    )
            }
            Action::Restart => {
                self.can_stop && matches!(state, RuntimeState::Running | RuntimeState::Active)
            }
            Action::Pause => self.can_pause && self.running(),
            Action::Resume => self.can_pause && state == RuntimeState::Paused,
            Action::Enable => matches!(startup, StartupMode::Disabled | StartupMode::Manual),
            Action::Disable => matches!(startup, StartupMode::Enabled | StartupMode::Manual),
            Action::Manual => matches!(startup, StartupMode::Enabled | StartupMode::Disabled),
        }
    }

    pub fn matches(&self, query: &str) -> bool {
        [
            &self.id,
            &self.name,
            &self.description,
            &self.state,
            &self.startup,
            &self.definition,
        ]
        .iter()
        .any(|value| value.to_lowercase().contains(query))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Start,
    Stop,
    Restart,
    Pause,
    Resume,
    Enable,
    Disable,
    Manual,
}

impl Action {
    pub const ALL: [Self; 8] = [
        Self::Start,
        Self::Stop,
        Self::Restart,
        Self::Pause,
        Self::Resume,
        Self::Enable,
        Self::Disable,
        Self::Manual,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Start => "Start",
            Self::Stop => "Stop",
            Self::Restart => "Restart",
            Self::Pause => "Pause",
            Self::Resume => "Resume",
            Self::Enable => "Enable startup",
            Self::Disable => "Disable startup",
            Self::Manual => "Set manual startup",
        }
    }

    pub fn explanation(self) -> &'static str {
        match self {
            Self::Start => "Start this service now.",
            Self::Stop if cfg!(target_os = "macos") => {
                "Unload this job. Applications that depend on it may be interrupted. It can load again when started or at the next login / boot."
            }
            Self::Stop => "Stop this service. Applications that depend on it may be interrupted.",
            Self::Restart => {
                "Interrupt and restart this service. Applications that depend on it may be interrupted."
            }
            Self::Pause => {
                "Pause this service. Applications that depend on it may stop responding."
            }
            Self::Resume => "Resume this paused service.",
            Self::Enable => {
                "Allow automatic startup (Windows: Automatic). This does not start the service now or change its launch conditions."
            }
            Self::Disable => {
                "Disable future automatic startup. This does not stop a running service. Other applications may depend on this service."
            }
            Self::Manual => "Set Windows startup to Manual. This does not stop a running service.",
        }
    }
}

#[derive(Clone)]
pub struct Request {
    pub scope: Scope,
    pub service: Service,
    pub action: Action,
}

pub fn validate(request: &Request, current: &[Service]) -> Result<(), String> {
    if request.service.id.is_empty()
        || request.service.id.starts_with('-')
        || request.service.id.chars().any(char::is_control)
        || request.service.id.contains('/')
    {
        return Err("Invalid service identity.".into());
    }
    let fresh = current
        .iter()
        .find(|service| service.id == request.service.id)
        .ok_or("The service no longer exists. Refresh and try again.")?;
    if fresh != &request.service {
        return Err(
            "The service changed since it was selected. Review its refreshed state and try again."
                .into(),
        );
    }
    if !fresh.allows(request.action) {
        return Err("This action is unavailable in the service's current state.".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn fixture() -> Service {
        Service {
            manager: Manager::Windows,
            id: "vijideck-fixture".into(),
            name: "Disposable service".into(),
            description: "Fixture only".into(),
            state: "Running".into(),
            startup: "Automatic".into(),
            pid: Some(42),
            definition: "fixture.exe".into(),
            can_stop: true,
            can_pause: false,
            loaded: true,
        }
    }

    #[test]
    fn capabilities_follow_manager_semantics() {
        let mut service = fixture();
        service.state = "Stopped".into();
        service.startup = "Disabled".into();
        service.manager = Manager::Launchd;
        assert!(!service.allows(Action::Manual));
        assert!(!service.allows(Action::Start));
        assert!(!service.allows(Action::Pause));
        service.manager = Manager::Windows;
        assert!(service.allows(Action::Manual));
        assert!(!service.allows(Action::Start));
        service.manager = Manager::Systemd;
        service.startup = "disabled".into();
        assert!(service.allows(Action::Start));
        for startup in ["masked", "masked-runtime"] {
            service.startup = startup.into();
            assert!(!service.allows(Action::Start));
        }
        service.state = "unexpected".into();
        assert!(!service.allows(Action::Stop));
    }
    #[test]
    fn rejects_stale_missing_and_unsupported_targets() {
        let service = fixture();
        let mut request = Request {
            scope: Scope::System,
            service: service.clone(),
            action: Action::Stop,
        };
        assert!(validate(&request, std::slice::from_ref(&service)).is_ok());
        let mut changed = service.clone();
        changed.pid = Some(43);
        assert!(validate(&request, &[changed]).is_err());
        assert!(validate(&request, &[]).is_err());
        request.action = Action::Pause;
        assert!(validate(&request, std::slice::from_ref(&service)).is_err());
        request.service.id = "--all".into();
        assert!(validate(&request, &[request.service.clone()]).is_err());
    }

    #[test]
    fn capabilities_distinguish_runtime_from_startup_and_templates() {
        let mut service = fixture();
        service.startup = "Disabled".into();
        assert!(service.allows(Action::Stop));
        assert!(service.allows(Action::Enable));
        assert!(!service.allows(Action::Disable));
        service.state = "Stopped".into();
        assert!(!service.allows(Action::Start));
        service.manager = Manager::Systemd;
        service.startup = "static".into();
        assert!(service.allows(Action::Start));
        assert!(!service.allows(Action::Enable));
        service.id = "fixture@.service".into();
        assert!(!service.allows(Action::Start));
        assert!(service.matches("fixture"));
        service.state = "active / exited".into();
        assert!(service.allows(Action::Stop));
        service.manager = Manager::Windows;
        service.state = "Paused".into();
        service.can_pause = true;
        assert!(service.allows(Action::Resume));
        assert!(!service.allows(Action::Pause));
    }
}
