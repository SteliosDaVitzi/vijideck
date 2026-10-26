pub struct Request {
    pub source: usize,
    pub minutes: u32,
    pub range: Option<(String, String)>,
}

pub fn startup_request() -> Option<Request> {
    parse_request(&std::env::args().skip(1).collect::<Vec<_>>())
}

fn parse_request(args: &[String]) -> Option<Request> {
    if args.first()?.as_str() != "--open-logs" {
        return None;
    }
    let source = args.get(1)?.parse::<usize>().ok()?;
    let minutes = args.get(2)?.parse::<u32>().ok()?;
    if source >= crate::logs::sources().len() || !(1..=1440).contains(&minutes) {
        return None;
    }
    let range = match args.len() {
        3 => None,
        5 => {
            let start = chrono::DateTime::parse_from_rfc3339(&args[3]).ok()?;
            let end = chrono::DateTime::parse_from_rfc3339(&args[4]).ok()?;
            if start > end {
                return None;
            }
            Some((start.to_rfc3339(), end.to_rfc3339()))
        }
        _ => return None,
    };
    Some(Request {
        source,
        minutes,
        range,
    })
}

#[cfg(windows)]
pub fn open_administrator_window(request: Request) -> Result<(), String> {
    use std::{mem::size_of, os::windows::ffi::OsStrExt};
    use windows_sys::Win32::{
        Foundation::{ERROR_CANCELLED, GetLastError},
        System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize},
        UI::{
            Shell::{SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SHELLEXECUTEINFOW, ShellExecuteExW},
            WindowsAndMessaging::SW_SHOWNORMAL,
        },
    };
    let executable = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut args = vec![
        "--open-logs".into(),
        request.source.to_string(),
        request.minutes.to_string(),
    ];
    if let Some((start, end)) = request.range {
        args.extend([start, end]);
    }
    // Normalize the only accepted arguments before constructing a Windows command line.
    let normalized = parse_request(&args).ok_or("Invalid log access request")?;
    let mut parameters = format!("--open-logs {} {}", normalized.source, normalized.minutes);
    if let Some((start, end)) = normalized.range {
        parameters.push_str(&format!(" {start} {end}"));
    }
    let executable: Vec<u16> = executable
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    let parameters: Vec<u16> = parameters.encode_utf16().chain(Some(0)).collect();
    let verb: Vec<u16> = "runas".encode_utf16().chain(Some(0)).collect();
    // The user explicitly requests this separate administrator instance in the UI.
    unsafe {
        let initialized = CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32) >= 0;
        let mut info: SHELLEXECUTEINFOW = std::mem::zeroed();
        info.cbSize = size_of::<SHELLEXECUTEINFOW>() as u32;
        info.fMask = SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI;
        info.lpVerb = verb.as_ptr();
        info.lpFile = executable.as_ptr();
        info.lpParameters = parameters.as_ptr();
        info.nShow = SW_SHOWNORMAL;
        let result = ShellExecuteExW(&mut info);
        let error = if result == 0 {
            Some(GetLastError())
        } else {
            None
        };
        if initialized {
            CoUninitialize();
        }
        match error {
            None => Ok(()),
            Some(ERROR_CANCELLED) => Err(
                "Administrator access was cancelled. You can keep using accessible logs.".into(),
            ),
            Some(code) => Err(format!(
                "Could not open an administrator window: {}",
                std::io::Error::from_raw_os_error(code as i32)
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_forwarded_log_query_and_rejects_extra_arguments() {
        let args = [
            "--open-logs",
            "1",
            "60",
            "2026-10-26T08:00:00+02:00",
            "2026-10-26T09:59:59+02:00",
        ]
        .map(str::to_owned);
        let request = parse_request(&args).unwrap();
        assert_eq!(request.source, 1);
        assert!(request.range.is_some());
        let mut invalid = args.to_vec();
        invalid.push("arbitrary-command".into());
        assert!(parse_request(&invalid).is_none());
        invalid = args.to_vec();
        invalid[3] = "'; command".into();
        assert!(parse_request(&invalid).is_none());
    }
}
