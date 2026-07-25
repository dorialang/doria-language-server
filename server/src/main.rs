use std::process::ExitCode;

fn main() -> ExitCode {
    doria_language_server::run_cli(std::env::args().skip(1))
}
