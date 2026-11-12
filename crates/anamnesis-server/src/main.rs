use clap::Parser;

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let cli = anamnesis_server::cli::Cli::parse();
    let bare = cli.is_bare();
    match anamnesis_server::cli::run(cli).await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("anamnesis: {e:#}");
            if bare {
                hold_console();
            }
            std::process::ExitCode::FAILURE
        }
    }
}

/// Started without a command (a double-click or a start-menu shortcut), windows closes
/// the console window as soon as the process exits; keep it open so the error can be
/// read. Only when stdin is a terminal, so scripts never block.
fn hold_console() {
    use std::io::IsTerminal;
    if cfg!(windows) && std::io::stdin().is_terminal() {
        eprintln!("press enter to close this window");
        let _ = std::io::stdin().read_line(&mut String::new());
    }
}
