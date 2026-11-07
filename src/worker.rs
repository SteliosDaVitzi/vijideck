//! Completion handling shared by finite background jobs. Streams retain their own events.
use std::sync::mpsc::{self, Receiver, TryRecvError};

pub fn receive<T>(rx: &Receiver<T>) -> Option<Result<T, String>> {
    match rx.try_recv() {
        Ok(value) => Some(Ok(value)),
        Err(TryRecvError::Empty) => None,
        Err(TryRecvError::Disconnected) => Some(Err(
            "Worker stopped unexpectedly. Refresh before retrying; an action may have completed."
                .into(),
        )),
    }
}

pub fn poll<T>(rx: &Receiver<Result<T, String>>) -> Option<Result<T, String>> {
    receive(rx).map(|result| result.and_then(|value| value))
}

pub fn spawn<T: Send + 'static>(
    task: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Receiver<Result<T, String>> {
    let (tx, rx) = mpsc::channel();
    let failure = tx.clone();
    if let Err(error) = std::thread::Builder::new()
        .name("background-job".into())
        .spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(task))
                .unwrap_or_else(|_| {
                    Err(
                        "Worker failed. Refresh before retrying; an action may have completed."
                            .into(),
                    )
                });
            let _ = tx.send(result);
        })
    {
        let _ = failure.send(Err(format!("Could not start worker: {error}")));
    }
    rx
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn distinguishes_pending_completion_and_disconnect() {
        let (tx, rx) = mpsc::channel();
        assert!(receive(&rx).is_none());
        tx.send(42).unwrap();
        assert_eq!(receive(&rx), Some(Ok(42)));
        drop(tx);
        assert!(receive(&rx).unwrap().is_err());
    }
    #[test]
    fn panics_produce_completion_errors() {
        let rx = spawn::<()>(|| panic!("fixture"));
        assert!(
            rx.recv_timeout(std::time::Duration::from_secs(2))
                .unwrap()
                .is_err()
        );
    }
}
