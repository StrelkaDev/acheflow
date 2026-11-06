fn main() -> Result<(), Box<dyn std::error::Error>> {
    acheflow_server::run_from(std::env::args_os(), None)
}
