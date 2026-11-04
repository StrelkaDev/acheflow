#[cfg(not(target_arch = "wasm32"))]
fn main() -> std::process::ExitCode {
    match acheflow_ui::launch_native() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            std::process::ExitCode::FAILURE
        }
    }
}
#[cfg(target_arch = "wasm32")]
fn main() {
    acheflow_ui::launch();
}
