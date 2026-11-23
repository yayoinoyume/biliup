use biliup_cli::entry;
use biliup_cli::server::errors::AppResult;

#[tokio::main]
async fn main() -> AppResult<()> {
    biliup_relay::install_crypto_provider();
    let cli = entry::parse(std::env::args_os()).unwrap_or_else(|e| e.exit());
    entry::run(cli).await
}
