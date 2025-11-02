//! Typo CLI - Voice-controlled AI assistant

mod app;
mod audio;
#[cfg(target_os = "macos")]
mod keyboard;

use anyhow::Result;
use clap::Parser;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// OpenAI API key (or set OPENAI_API_KEY env var)
    #[arg(long, env = "OPENAI_API_KEY")]
    api_key: String,

    /// OpenAI model to use
    #[arg(long, default_value = "gpt-realtime-2025-08-28")]
    model: String,

    /// Log level (trace, debug, info, warn, error)
    #[arg(long, default_value = "info")]
    log_level: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Initialize tracing
    let log_filter = args.log_level.as_str();
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| log_filter.into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    tracing::info!("🐛 typo is here to do your bidding");
    println!("{}", "=".repeat(34));

    // Run the app
    app::run(args.api_key, args.model).await?;

    Ok(())
}
