use std::process::ExitCode;

fn main() -> ExitCode {
    match std::env::args().nth(1).as_deref() {
        Some("--version" | "-V") => {
            println!(
                "doria-lsp {} (Doria {})",
                doria_language_server::SERVER_VERSION,
                doria_language_server::toolchain_version(),
            );
            return ExitCode::SUCCESS;
        }
        Some("--help" | "-h") => {
            println!("doria-lsp [--version]\n\nWithout arguments, starts the Doria language server over stdio.");
            return ExitCode::SUCCESS;
        }
        Some(argument) => {
            eprintln!("unknown argument: {argument}");
            return ExitCode::from(2);
        }
        None => {}
    }

    match doria_language_server::run_stdio() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}
