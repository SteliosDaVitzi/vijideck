//! Shared process identity policy and native termination.
use sysinfo::{Pid, ProcessesToUpdate, System};
#[derive(Clone, Copy)]
pub struct Identity {
    pub pid: u32,
    pub start: u64,
}
pub fn with_verified<T>(
    identity: Identity,
    action: impl FnOnce(&sysinfo::Process) -> Result<T, String>,
) -> Result<T, String> {
    if identity.pid <= 4 || identity.pid == std::process::id() {
        return Err("This process is protected by VijiDeck.".into());
    }
    let mut system = System::new();
    system.refresh_processes(
        ProcessesToUpdate::Some(&[Pid::from_u32(identity.pid)]),
        true,
    );
    let process = system
        .process(Pid::from_u32(identity.pid))
        .ok_or("The process has already exited.")?;
    if identity.start == 0 || process.start_time() != identity.start {
        return Err("Process identity changed. Refresh and select it again.".into());
    }
    action(process)
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
