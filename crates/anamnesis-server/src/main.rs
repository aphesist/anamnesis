use clap::Parser;

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let cli = anamnesis_server::cli::Cli::parse();
    match anamnesis_server::cli::run(cli).await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("anamnesis: {e:#}");
            std::process::ExitCode::FAILURE
        }
    }
}
