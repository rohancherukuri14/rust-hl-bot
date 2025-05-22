use anyhow::Result;
use tracing::{info, error};
use tracing_subscriber;

mod config;
mod database;
mod telegram;
mod hyperliquid;
mod coordinator;

use config::Config;
use telegram::TelegramBot;
use hyperliquid::{HyperliquidClient, WebSocketManager};
use coordinator::TradeCoordinator;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("hyperliquid_telegram_bot=debug,info")
        .init();

    info!("Starting Hyperliquid Telegram Bot");

    let config = Config::load()?;
    info!("config loaded");

    let db = database::init(&config.database).await?;
    info!("connected to db");

    let hyperliquid_client = HyperliquidClient::new(config.hyperliquid.clone());
    info!("hl client init success");

    let ws_manager = WebSocketManager::new(config.hyperliquid.websocket_url.clone());
    info!("hl ws init success");

    // Create dummy telegram bot for coordinator
    let dummy_bot = TelegramBot::new(
        config.telegram.bot_token.clone(),
        db.clone(),
        hyperliquid_client.clone(),
        tokio::sync::mpsc::unbounded_channel().0
    );

    let (coordinator, event_sender, event_receiver) = TradeCoordinator::new(
        db.clone(),
        dummy_bot,
        ws_manager,
        config.clone()
    );
    info!("coordinator ready");

    let telegram_bot = TelegramBot::new(
        config.telegram.bot_token.clone(), 
        db.clone(),
        hyperliquid_client,
        event_sender
    );
    info!("tg bot ready");

    tokio::spawn(async move {
        if let Err(e) = coordinator.start(event_receiver).await {
            error!("coordinator error: {}", e);
        }
    });

    telegram_bot.start().await?;

    Ok(())
}