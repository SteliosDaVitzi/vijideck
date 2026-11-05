//! Bounded capture for short-lived external commands. Streaming processes have
//! their own lifecycle (for example Docker logs and interactive terminals).
use std::{
    fmt,
    io::{self, Read},
    process::{Child, Command, Output, Stdio},
    sync::mpsc::{self, Receiver, TryRecvError},
    time::{Duration, Instant},
};

#[derive(Clone, Copy)]
pub(crate) struct Limits {
    pub timeout: Duration,
    /// Maximum bytes retained from each output stream.
    pub output_bytes: usize,
}

#[derive(Debug)]
pub(crate) enum Error {
    Io(io::Error),
    Timeout,
    OutputLimit,
    ReaderDisconnected,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => error.fmt(f),
            Self::Timeout => f.write_str("command timed out"),
            Self::OutputLimit => f.write_str("command output exceeded its byte limit"),
            Self::ReaderDisconnected => f.write_str("command output reader disconnected"),
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

// Reap the direct child on every error path, including failures reading a pipe.
struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

type PipeResult = Result<Vec<u8>, Error>;

fn reader(pipe: impl Read + Send + 'static, limit: usize) -> Receiver<PipeResult> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let result = (|| {
            let mut bytes = Vec::new();
            // Read one extra byte to distinguish exactly-at-limit from overflow.
            pipe.take((limit as u64).saturating_add(1))
                .read_to_end(&mut bytes)?;
            if bytes.len() > limit {
                Err(Error::OutputLimit)
            } else {
                Ok(bytes)
            }
        })();
        let _ = tx.send(result);
    });
    rx
}

fn receive(rx: &Receiver<PipeResult>, bytes: &mut Option<Vec<u8>>) -> Result<(), Error> {
    if bytes.is_none() {
        match rx.try_recv() {
            Ok(result) => *bytes = Some(result?),
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => return Err(Error::ReaderDisconnected),
        }
    }
    Ok(())
}

/// Capture both pipes concurrently, applying one deadline to process exit AND
/// pipe completion. A descendant may inherit a pipe after the direct child exits;
/// never join a blocking reader on the calling thread. Reader threads can outlive
/// a timed-out command until inherited pipe handles close. This does not supervise
/// or terminate descendant processes.
pub(crate) fn capture(mut command: Command, limits: Limits) -> Result<Output, Error> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let deadline = Instant::now() + limits.timeout;
    let mut child = ChildGuard(command.spawn()?);
    let stdout = reader(
        child.0.stdout.take().expect("stdout was piped"),
        limits.output_bytes,
    );
    let stderr = reader(
        child.0.stderr.take().expect("stderr was piped"),
        limits.output_bytes,
    );
    let (mut out, mut err, mut status) = (None, None, None);
    loop {
        receive(&stdout, &mut out)?;
        receive(&stderr, &mut err)?;
        if status.is_none() {
            status = child.0.try_wait()?;
        }
        if let (Some(status), Some(stdout), Some(stderr)) = (status, out.as_mut(), err.as_mut()) {
            return Ok(Output {
                status,
                stdout: std::mem::take(stdout),
                stderr: std::mem::take(stderr),
            });
        }
        if Instant::now() >= deadline {
            return Err(Error::Timeout);
        }
        std::thread::sleep(
            Duration::from_millis(10).min(deadline.saturating_duration_since(Instant::now())),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_distinguish_exact_size_overflow_and_read_failure() {
        let rx = reader(&b"abcd"[..], 4);
        assert_eq!(rx.recv().unwrap().unwrap(), b"abcd");
        assert!(matches!(
            reader(&b"abcde"[..], 4).recv().unwrap(),
            Err(Error::OutputLimit)
        ));
        struct Broken;
        impl Read for Broken {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(io::Error::other("broken pipe"))
            }
        }
        assert!(matches!(
            reader(Broken, 4).recv().unwrap(),
            Err(Error::Io(_))
        ));
    }

    #[cfg(unix)]
    fn shell(script: &str, timeout: Duration, output_bytes: usize) -> Result<Output, Error> {
        let mut command = Command::new("sh");
        command.args(["-c", script]);
        capture(command, Limits { timeout, output_bytes })
    }

    #[cfg(unix)]
    #[test]
    fn captures_both_large_pipes_and_preserves_failure_status() {
        let output = shell(
            "head -c 131072 /dev/zero; head -c 131072 /dev/zero >&2; exit 7",
            Duration::from_secs(5),
            131072,
        ).unwrap();
        assert_eq!(output.stdout.len(), 131072);
        assert_eq!(output.stderr.len(), 131072);
        assert_eq!(output.status.code(), Some(7));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_output_overflow_and_times_out_running_children() {
        assert!(matches!(
            shell("head -c 8192 /dev/zero", Duration::from_secs(5), 4096),
            Err(Error::OutputLimit)
        ));
        assert!(matches!(
            shell("exec sleep 5", Duration::from_millis(50), 4096),
            Err(Error::Timeout)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn deadline_includes_pipes_inherited_by_descendants() {
        let start = Instant::now();
        assert!(matches!(
            shell("sleep 2 & exit 0", Duration::from_millis(50), 4096),
            Err(Error::Timeout)
        ));
        assert!(start.elapsed() < Duration::from_secs(1));
    }
}
