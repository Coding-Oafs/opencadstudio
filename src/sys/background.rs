//! Run blocking work without occupying the async executor while it completes.

pub async fn run<T, F>(work: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let (tx, rx) = iced::futures::channel::oneshot::channel();
    std::thread::Builder::new()
        .name("ocs-background".into())
        .spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(work))
                .map_err(|_| "Background operation failed unexpectedly. Please retry.".to_string());
            let _ = tx.send(result);
        })
        .map_err(|error| format!("Could not start background operation: {error}"))?;
    rx.await
        .map_err(|_| "Background operation ended without a result.".to_string())?
}

#[cfg(test)]
mod tests {
    use super::run;

    #[test]
    fn returns_results_and_contains_worker_panics() {
        iced::futures::executor::block_on(async {
            assert_eq!(run(|| 42).await.unwrap(), 42);
            assert!(run(|| panic!("failed worker")).await.is_err());
        });
    }

    #[test]
    fn waiting_for_work_yields_the_executor() {
        use iced::futures::{executor::block_on, future::join};
        let (tx, rx) = std::sync::mpsc::channel();
        let (result, ()) = block_on(join(
            run(move || rx.recv_timeout(std::time::Duration::from_secs(2))),
            async move {
                tx.send(42).unwrap();
            },
        ));
        assert_eq!(result.unwrap().unwrap(), 42);
    }
}
