//! Native credential prompts must not stall the UI executor.
pub(crate) async fn run<T: Send + 'static>(
    operation: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tokio::task::spawn_blocking(operation)
        .await
        .map_err(|_| "Native credential operation could not finish.".to_owned())?
}

#[cfg(test)]
mod tests {
    #[tokio::test(flavor = "current_thread")]
    async fn os_prompt_wait_leaves_the_ui_executor_available() {
        let (release, pending) = std::sync::mpsc::channel();
        let storage = super::run(move || {
            pending
                .recv_timeout(std::time::Duration::from_secs(1))
                .map_err(|_| "The UI could not respond while credentials waited.".to_owned())
        });
        let ui = async move {
            tokio::task::yield_now().await;
            let _ = release.send(());
        };
        let (result, ()) = tokio::join!(storage, ui);
        assert!(result.is_ok(), "{}", result.unwrap_err());
    }
}
