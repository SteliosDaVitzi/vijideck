//! Application navigation. Exhaustive matches keep new tools visible to the compiler.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Tool {
    Ports,
    Cleanup,
    Overview,
    Logs,
    Processes,
    Environment,
    Docker,
    Services,
}

impl Tool {
    pub(super) fn title(self) -> &'static str {
        match self {
            Self::Ports => "Port manager",
            Self::Cleanup => "Disk cleanup",
            Self::Overview => "Overview",
            Self::Logs => "System logs",
            Self::Processes => "Processes",
            Self::Environment => "Developer environment",
            Self::Docker => "Docker",
            Self::Services => "Services",
        }
    }
}
