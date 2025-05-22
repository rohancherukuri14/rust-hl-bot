pub mod client;
pub mod websocket;

use serde::{Deserialize, Serialize};


#[derive(Serialize)]
pub struct InfoRequest {
    #[serde(rename = "type")]
    pub request_type: String,
}

#[derive(Debug, Deserialize)]
pub struct MetaAndAssetCtxsResponse {
    pub universe: Vec<AssetInfo>,
}

#[derive(Debug, Deserialize)]
pub struct AssetInfo {
    pub name: String,
    #[serde(rename = "isDelisted")]
    pub is_delisted: Option<bool>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct WsTrade {
    pub coin: String,
    pub side: String, 
    pub px: String, 
    pub sz: String, 
}

impl WsTrade {
    pub fn notional_usd(&self) -> anyhow::Result<f64> {
        let price: f64 = self.px.parse()?;
        let size: f64 = self.sz.parse()?;
        Ok(price * size)
    }
}

pub use client::HyperliquidClient;
pub use websocket::{WebSocketManager};