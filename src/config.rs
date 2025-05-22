use anyhow::Result;
use serde::{Deserialize, Serialize};
use config::{Config as ConfigBuilder, File};

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Config {
    pub telegram: TelegramConfig,
    pub hyperliquid: HyperliquidConfig,
    pub database: DatabaseConfig,
    pub defaults: DefaultsConfig,
    pub retry: RetryConfig,
    pub commands: CommandsConfig,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct TelegramConfig {
    pub bot_token: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct HyperliquidConfig {
    pub websocket_url: String,
    pub rest_api_url: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct DatabaseConfig {
    pub url: String,
    pub api_key: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct DefaultsConfig {
    pub default_symbol: String,
    pub min_trade_value_usd: f64,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct RetryConfig {
    pub max_attempts: u32,
    pub base_delay_ms: u64,
    pub max_delay_ms: u64,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct CommandsConfig {
    pub subscribe_command: String,
    pub unsubscribe_command: String,
    pub list_command: String,
    pub help_command: String,
}

impl Config {
    pub fn load() -> Result<Self> {
        let config = ConfigBuilder::builder()
            .add_source(File::with_name("config"))
            .build()?;

        let config: Config = config.try_deserialize()?;
        Ok(config)
    }
}