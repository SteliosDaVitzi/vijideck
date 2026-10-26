use netstat2::{
    AddressFamilyFlags as Af, ProtocolFlags as Proto, ProtocolSocketInfo, get_sockets_info,
};
use std::{collections::HashMap, net::SocketAddr};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, Signal, System, UpdateKind, Users};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SocketKey {
    pub protocol: &'static str,
    pub local: SocketAddr,
    pub remote: Option<SocketAddr>,
    pub pid: Option<u32>,
}

#[derive(Clone, Debug)]
pub struct Port {
    pub key: SocketKey,
    pub state: String,
}

#[derive(Clone, Debug)]
pub struct Process {
    pub pid: u32,
    pub name: String,
    pub executable: String,
    pub arguments: Vec<String>,
    pub cwd: String,
    pub parent: String,
    pub user: String,
    pub status: String,
    pub start: u64,
    pub uptime: u64,
    pub cpu: f32,
    pub memory: u64,
    pub virtual_memory: u64,
    pub read_bytes: u64,
    pub written_bytes: u64,
}

#[derive(Default)]
pub struct Snapshot {
    pub ports: Vec<Port>,
    pub processes: HashMap<u32, Process>,
    pub warnings: Vec<String>,
}

fn sockets() -> (Vec<Port>, Vec<String>) {
    let mut ports = Vec::new();
    let mut warnings = Vec::new();
    for (family, family_name) in [(Af::IPV4, "IPv4"), (Af::IPV6, "IPv6")] {
        for (protocol, protocol_name) in [(Proto::TCP, "TCP"), (Proto::UDP, "UDP")] {
            match get_sockets_info(family, protocol) {
                Ok(sockets) => {
                    for socket in sockets {
                        let (local, remote, state) = match socket.protocol_socket_info {
                            ProtocolSocketInfo::Tcp(t) => (
                                SocketAddr::new(t.local_addr, t.local_port),
                                Some(SocketAddr::new(t.remote_addr, t.remote_port)),
                                format!("{:?}", t.state),
                            ),
                            ProtocolSocketInfo::Udp(u) => (
                                SocketAddr::new(u.local_addr, u.local_port),
                                None,
                                "Bound".into(),
                            ),
                        };
                        let owners: Vec<_> = if socket.associated_pids.is_empty() {
                            vec![None]
                        } else {
                            socket.associated_pids.into_iter().map(Some).collect()
                        };
                        for pid in owners {
                            ports.push(Port {
                                key: SocketKey {
                                    protocol: protocol_name,
                                    local,
                                    remote,
                                    pid,
                                },
                                state: state.clone(),
                            });
                        }
                    }
                }
                Err(e) => warnings.push(format!("{family_name} {protocol_name}: {e}")),
            }
        }
    }
    ports.sort_by_key(|p| {
        (
            p.key.local.port(),
            p.key.protocol,
            p.key.local,
            p.key.remote,
            p.key.pid,
        )
    });
    ports.dedup_by(|a, b| a.key == b.key && a.state == b.state);
    (ports, warnings)
}

pub fn scan(system: &mut System) -> Snapshot {
    let (ports, warnings) = sockets();

    // Keep only sysinfo's lightweight process inventory for the whole system.
    // Detailed command lines, paths, users, and counters are needed only for
    // processes currently attached to a socket.
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing(),
    );
    let pids: Vec<_> = ports
        .iter()
        .filter_map(|port| port.key.pid.map(Pid::from_u32))
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&pids),
        true,
        ProcessRefreshKind::nothing()
            .with_cpu()
            .with_memory()
            .with_disk_usage()
            .with_exe(UpdateKind::OnlyIfNotSet)
            .with_cmd(UpdateKind::Always)
            .with_cwd(UpdateKind::Always)
            .with_user(UpdateKind::OnlyIfNotSet),
    );
    let users = Users::new_with_refreshed_list();
    let mut processes = HashMap::new();
    for pid in ports.iter().filter_map(|p| p.key.pid) {
        if processes.contains_key(&pid) {
            continue;
        }
        if let Some(p) = system.process(Pid::from_u32(pid)) {
            let disk = p.disk_usage();
            let parent = p
                .parent()
                .map(|id| {
                    let name = system
                        .process(id)
                        .map(|p| p.name().to_string_lossy().into_owned())
                        .unwrap_or_default();
                    format!("{id}  {name}")
                })
                .unwrap_or_else(|| "Unavailable".into());
            processes.insert(
                pid,
                Process {
                    pid,
                    name: p.name().to_string_lossy().into_owned(),
                    executable: p.exe().map(|v| v.display().to_string()).unwrap_or_else(|| {
                        "Unavailable (access restricted or process exited)".into()
                    }),
                    arguments: p
                        .cmd()
                        .iter()
                        .map(|v| v.to_string_lossy().into_owned())
                        .collect(),
                    cwd: p
                        .cwd()
                        .map(|v| v.display().to_string())
                        .unwrap_or_else(|| "Unavailable".into()),
                    parent,
                    user: p
                        .user_id()
                        .and_then(|id| users.get_user_by_id(id))
                        .map(|u| u.name().to_owned())
                        .unwrap_or_else(|| "Unavailable".into()),
                    status: format!("{:?}", p.status()),
                    start: p.start_time(),
                    uptime: p.run_time(),
                    cpu: p.cpu_usage(),
                    memory: p.memory(),
                    virtual_memory: p.virtual_memory(),
                    read_bytes: disk.total_read_bytes,
                    written_bytes: disk.total_written_bytes,
                },
            );
        }
    }
    Snapshot {
        ports,
        processes,
        warnings,
    }
}

pub fn matches(port: &Port, process: Option<&Process>, query: &str) -> bool {
    let haystack = format!(
        "{} {} {} {} {} {}",
        port.key.protocol,
        port.key.local,
        port.key.remote.map(|r| r.to_string()).unwrap_or_default(),
        port.state,
        port.key.pid.map(|p| p.to_string()).unwrap_or_default(),
        process
            .map(|p| format!(
                "{} {} {} {} {}",
                p.name,
                p.executable,
                p.arguments.join(" "),
                p.user,
                p.cwd
            ))
            .unwrap_or_default()
    )
    .to_lowercase();
    query
        .split_whitespace()
        .all(|word| haystack.contains(&word.to_lowercase()))
}

#[derive(Clone)]
pub struct StopRequest {
    pub socket: SocketKey,
    pub start: u64,
    pub name: String,
    pub force: bool,
}

pub fn stop(request: &StopRequest) -> Result<String, String> {
    let pid = request
        .socket
        .pid
        .ok_or("No process owner is available for this socket.")?;
    if pid <= 4 || pid == std::process::id() {
        return Err("This process is protected by VijiDeck.".into());
    }
    let mut system = System::new();
    system.refresh_processes(ProcessesToUpdate::Some(&[Pid::from_u32(pid)]), true);
    let p = system
        .process(Pid::from_u32(pid))
        .ok_or("The process has already exited.")?;
    if request.start == 0 || p.start_time() != request.start {
        return Err("Process identity changed. Refresh and select it again.".into());
    }
    let (current, warnings) = sockets();
    if !current.iter().any(|p| p.key == request.socket) {
        return Err(format!(
            "Socket ownership could not be verified. Refresh and try again. {}",
            warnings.join("; ")
        ));
    }
    let sent = if request.force {
        force_kill(p, request.start)?
    } else {
        p.kill_with(Signal::Term).ok_or(
            "Graceful termination is unsupported on this platform. Use Force kill if needed.",
        )?
    };
    if sent {
        Ok(format!(
            "{} signal sent to {} (PID {pid}). Refreshing to check the result.",
            if request.force { "Kill" } else { "Termination" },
            request.name
        ))
    } else {
        Err("The OS rejected the request. The process may have exited or require administrator/root permissions.".into())
    }
}

#[cfg(not(windows))]
pub(crate) fn force_kill(process: &sysinfo::Process, _: u64) -> Result<bool, String> {
    Ok(process.kill())
}

// Use a native process handle rather than shelling out to taskkill. The same
// handle is used to verify creation time and terminate, avoiding PID reuse here.
#[cfg(windows)]
pub(crate) fn force_kill(process: &sysinfo::Process, expected_start: u64) -> Result<bool, String> {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, FILETIME, HANDLE},
        System::Threading::{
            GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE,
            TerminateProcess,
        },
    };
    struct Handle(HANDLE);
    impl Drop for Handle {
        fn drop(&mut self) {
            // SAFETY: This handle is uniquely owned and was returned by OpenProcess.
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
    // SAFETY: OpenProcess takes a numeric PID and returns an owned handle or null.
    let raw = unsafe {
        OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_TERMINATE,
            0,
            process.pid().as_u32(),
        )
    };
    if raw.is_null() {
        return Err(format!(
            "Cannot open process for termination: {}",
            std::io::Error::last_os_error()
        ));
    }
    let handle = Handle(raw);
    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    // SAFETY: All output pointers refer to valid FILETIMEs and handle is live.
    if unsafe { GetProcessTimes(handle.0, &mut creation, &mut exit, &mut kernel, &mut user) } == 0 {
        return Err(format!(
            "Cannot verify process creation time: {}",
            std::io::Error::last_os_error()
        ));
    }
    let ticks = (u64::from(creation.dwHighDateTime) << 32) | u64::from(creation.dwLowDateTime);
    let unix_seconds = (ticks / 10_000_000).saturating_sub(11_644_473_600);
    if unix_seconds != expected_start {
        return Err("Process identity changed. Refresh and select it again.".into());
    }
    // SAFETY: The verified, live handle was opened with PROCESS_TERMINATE rights.
    if unsafe { TerminateProcess(handle.0, 1) } == 0 {
        return Err(format!(
            "Termination failed: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn finds_real_listener_and_searches() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let snapshot = scan(&mut System::new_all());
        let port = snapshot
            .ports
            .iter()
            .find(|p| p.key.local == address && p.key.pid == Some(std::process::id()))
            .expect("test listener with owning PID");
        assert!(matches(port, None, &format!("tcp {}", address.port())));
        assert!(!matches(port, None, "udp"));
        assert!(snapshot.processes.contains_key(&std::process::id()));
    }
    #[test]
    fn protects_self_and_system() {
        for pid in [0, 1, 4, std::process::id()] {
            let request = StopRequest {
                socket: SocketKey {
                    protocol: "TCP",
                    local: "127.0.0.1:1".parse().unwrap(),
                    remote: None,
                    pid: Some(pid),
                },
                start: 1,
                name: "test".into(),
                force: true,
            };
            assert!(stop(&request).unwrap_err().contains("protected"));
        }
    }

    // Invoked only by the parent test in a separate disposable process.
    #[test]
    #[ignore]
    fn child_listener() {
        let Some(path) = std::env::var_os("VIJIDECK_TEST_PORT_FILE") else {
            return;
        };
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        std::fs::write(path, listener.local_addr().unwrap().to_string()).unwrap();
        std::thread::sleep(std::time::Duration::from_secs(30));
        drop(listener);
    }

    #[test]
    fn kills_owned_child_and_rejects_stale_identity() {
        use std::{
            process::{Child, Command, Stdio},
            time::{Duration, Instant},
        };
        struct ChildGuard(Child, std::path::PathBuf);
        impl Drop for ChildGuard {
            fn drop(&mut self) {
                self.0.kill().ok();
                self.0.wait().ok();
                std::fs::remove_file(&self.1).ok();
            }
        }
        let file = std::env::temp_dir().join(format!(
            "vijideck-port-test-{}-{}.txt",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let child = Command::new(std::env::current_exe().unwrap())
            .args(["--ignored", "--exact", "ports::tests::child_listener"])
            .env("VIJIDECK_TEST_PORT_FILE", &file)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut guard = ChildGuard(child, file);
        let deadline = Instant::now() + Duration::from_secs(10);
        let address: SocketAddr = loop {
            if let Ok(text) = std::fs::read_to_string(&guard.1)
                && let Ok(address) = text.parse()
            {
                break address;
            }
            assert!(Instant::now() < deadline, "child listener did not start");
            std::thread::sleep(Duration::from_millis(50));
        };
        let snapshot = scan(&mut System::new_all());
        let port = snapshot
            .ports
            .iter()
            .find(|p| p.key.local == address && p.key.pid == Some(guard.0.id()))
            .expect("child socket owner");
        let process = &snapshot.processes[&guard.0.id()];
        assert!(
            !process.arguments.is_empty(),
            "command arguments must be refreshed"
        );
        let mut request = StopRequest {
            socket: port.key.clone(),
            start: process.start + 1,
            name: process.name.clone(),
            force: true,
        };
        assert!(stop(&request).unwrap_err().contains("identity changed"));
        assert!(guard.0.try_wait().unwrap().is_none());
        request.start = process.start;
        let mut wrong_socket = request.clone();
        wrong_socket.socket.local.set_port(0);
        assert!(stop(&wrong_socket).unwrap_err().contains("ownership"));
        assert!(guard.0.try_wait().unwrap().is_none());
        assert!(stop(&request).unwrap().contains("signal sent"));
        let deadline = Instant::now() + Duration::from_secs(5);
        while guard.0.try_wait().unwrap().is_none() {
            assert!(Instant::now() < deadline, "child did not exit after kill");
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(
            std::net::TcpListener::bind(address).is_ok(),
            "port must be released"
        );
    }
}
