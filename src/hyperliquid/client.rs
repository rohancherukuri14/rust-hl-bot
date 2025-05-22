use anyhow::Result;
use reqwest::Client;
use std::collections::HashSet;
use tokio::time::{Duration, Instant};
use tracing::{info, error, warn};
use crate::config::HyperliquidConfig;
use super::{InfoRequest, MetaAndAssetCtxsResponse};

#[derive(Clone)]
pub struct HyperliquidClient {
    client: Client,
    config: HyperliquidConfig,
    // cache coins, refresh every 6 hours
    valid_coins: Option<HashSet<String>>,
    last_fetched: Option<Instant>,
}

impl HyperliquidClient {
    pub fn new(config: HyperliquidConfig) -> Self {
        let client = Client::new();
        
        HyperliquidClient {
            client,
            config,
            valid_coins: None,
            last_fetched: None,
        }
    }

    async fn fetch_valid_coins(&mut self) -> Result<()> {
        info!("fetching coins from hl...");

        let request_body = InfoRequest {
            request_type: "metaAndAssetCtxs".to_string(),
        };

        let response = self
            .client
            .post(&format!("{}/info", self.config.rest_api_url))
            .header("Content-Type", "application/json")
            .json(&request_body)
            .send()
            .await?;

        if !response.status().is_success() {
            error!("hl api request failed, status: {}", response.status());
            return Err(anyhow::anyhow!("hl api error"));
        }

        let response_text = response.text().await?;
        
        let json_value: serde_json::Value = serde_json::from_str(&response_text)?;
        
        let array = json_value.as_array()
            .ok_or_else(|| anyhow::anyhow!("bad response from hl"))?;
        
        let meta_object = array
            .iter()
            .find(|obj| obj.get("universe").is_some())
            .ok_or_else(|| anyhow::anyhow!("bad response from hl"))?;
        
        let meta_response: MetaAndAssetCtxsResponse = serde_json::from_value(meta_object.clone())?;

        let coins: HashSet<String> = meta_response
            .universe
            .iter()
            .filter(|asset| !asset.is_delisted.unwrap_or(false)) // no delists
            .map(|asset| asset.name.to_uppercase())
            .collect();

        info!("fetched {} valid coins from hl", coins.len());
        self.valid_coins = Some(coins);
        self.last_fetched = Some(Instant::now());

        Ok(())
    }

    pub async fn coin_exists(&mut self, coin: &str) -> Result<bool> {
        // stale cache after 6 hours
        let needs_refresh = match self.last_fetched {
            None => true,
            Some(last_time) => last_time.elapsed() >= Duration::from_secs(6 * 60 * 60),
        };

        if needs_refresh {
            if let Err(e) = self.fetch_valid_coins().await {
                error!("couldn't fetch valid coins: {}", e);
                return Err(e);
            }
        }

        let coin_upper = coin.to_uppercase();
        let exists = self
            .valid_coins
            .as_ref()
            .expect("no valid coins") 
            .contains(&coin_upper);

        if exists {
            info!("{} is valid", coin_upper);
        } else {
            warn!("{} is not available on hl", coin_upper);
        }

        Ok(exists)
    }
}