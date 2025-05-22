use anyhow::{Result, anyhow};
use serde::{Serialize, Deserialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use tokio::time::{sleep, Duration};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use futures_util::{SinkExt, StreamExt};
use tracing::{info, error, warn, debug};
use super::WsTrade;

#[derive(Debug, Deserialize)]
struct WsResponse {
    data: Vec<WsTrade>,
}
#[derive(Serialize)]
struct WsSubscription {
    method: String,
    subscription: WsSubscriptionData,
}

#[derive(Serialize)]
struct WsSubscriptionData {
    #[serde(rename = "type")]
    sub_type: String,
    coin: String,
}

#[derive(Debug)]
pub struct WebSocketHandle {
    coin: String,
    shutdown_tx: mpsc::Sender<()>,
}

impl WebSocketHandle {
    
    pub async fn shutdown(self) {
        let _ = self.shutdown_tx.send(()).await;
        info!("shutdown signal for {}", self.coin);
    }
}

pub struct WebSocketManager {
    websocket_url: String,
    active_websockets: Arc<RwLock<HashMap<String, WebSocketHandle>>>,
}

impl WebSocketManager {
    pub fn new(websocket_url: String) -> Self {
        WebSocketManager {
            websocket_url,
            active_websockets: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn start_trade_feed(
        &self, 
        coin: &str, 
        trade_sender: mpsc::UnboundedSender<WsTrade>
    ) -> anyhow::Result<WebSocketHandle> {
        let coin = coin.to_uppercase();
        
        {
            let websockets = self.active_websockets.read().await;
            if websockets.contains_key(&coin) {
                return Err(anyhow::anyhow!("ws alr exists for {}", coin));
            }
        }

        let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);
        let websocket_url = self.websocket_url.clone();
        let coin_clone = coin.clone();
        let active_websockets = self.active_websockets.clone();

        tokio::spawn(async move {
            let mut retry_count = 0;
            const MAX_RETRIES: u32 = 5;
            const BASE_DELAY: u64 = 1000;
            const MAX_DELAY: u64 = 30000;

            loop {
                if shutdown_rx.try_recv().is_ok() {
                    break;
                }

                info!("trying to connect to {} ws (attempt {})", coin_clone, retry_count + 1);

                match Self::websocket_connection(
                    &websocket_url, 
                    &coin_clone, 
                    trade_sender.clone(), 
                    &mut shutdown_rx
                ).await {
                    Ok(_) => {
                        break; //websocket ended
                    }
                    Err(e) => {
                        error!("ws connection for {} failed: {}", coin_clone, e);
                        retry_count += 1;
                        
                        if retry_count >= MAX_RETRIES {
                            error!("max retries reached for {}", coin_clone);
                            break;
                        }
                    }
                }

                let delay = std::cmp::min(BASE_DELAY * 2_u64.pow(retry_count), MAX_DELAY);
                let jitter = (delay as f64 * 0.1 * rand::random::<f64>()) as u64;
                let total_delay = delay + jitter;
                
                warn!("retrying {} ws in {}ms", coin_clone, total_delay);
                sleep(Duration::from_millis(total_delay)).await;
            }

            let mut websockets = active_websockets.write().await;
            websockets.remove(&coin_clone);
            info!("removed {} ws", coin_clone);
        });

        let handle = WebSocketHandle {
            coin: coin.clone(),
            shutdown_tx: shutdown_tx.clone(),
        };

        {
            let mut websockets = self.active_websockets.write().await;
            websockets.insert(coin.clone(), handle);
        }

        Ok(WebSocketHandle {
            coin,
            shutdown_tx,
        })
    }

    async fn websocket_connection(
        websocket_url: &str,
        coin: &str,
        trade_sender: mpsc::UnboundedSender<WsTrade>,
        shutdown_rx: &mut mpsc::Receiver<()>,
    ) -> anyhow::Result<()> {
        let (ws_stream, _) = connect_async(websocket_url).await?;
        let (mut ws_sender, mut ws_receiver) = ws_stream.split();

        let subscription = WsSubscription {
            method: "subscribe".to_string(),
            subscription: WsSubscriptionData {
                sub_type: "trades".to_string(),
                coin: coin.to_string(),
            },
        };

        let sub_message = serde_json::to_string(&subscription)?;
        ws_sender.send(Message::Text(sub_message)).await?;

        loop {
            tokio::select! {
                _ = shutdown_rx.recv() => {
                    let _ = ws_sender.close().await;
                    break;
                }
                
                message = ws_receiver.next() => {
                    match message {
                        Some(Ok(Message::Text(text))) => {
                            match serde_json::from_str::<WsResponse>(&text) {
                                Ok(ws_response) => {
                                    for trade in ws_response.data {
                                        
                                        if let Err(_) = trade_sender.send(trade) {
                                            warn!("receiver dropped, closing {} ws", coin);
                                            break;
                                        }
                                    }
                                }
                                Err(e) => {
                                    debug!("parse error: {} (error msg: {})", text, e);
                                }
                            }
                        }
                        Some(Ok(Message::Close(_))) => {
                            info!("ws closed by server for {}", coin);
                            break;
                        }
                        Some(Err(e)) => {
                            error!("ws error for {}: {}", coin, e);
                            return Err(anyhow::anyhow!("ws error: {}", e));
                        }
                        None => {
                            warn!("ws ended for {}", coin);
                            break;
                        }
                        _ => {
                            debug!("received non-text message for {}", coin);
                        }
                    }
                }
            }
        }

        Ok(())
    }

    pub async fn stop_trade_feed(&self, coin: &str) -> anyhow::Result<()> {
        let coin = coin.to_uppercase();
        
        let mut websockets = self.active_websockets.write().await;
        if let Some(handle) = websockets.remove(&coin) {
            handle.shutdown().await;
            Ok(())
        } else {
            warn!("no active ws for {}", coin);
            Err(anyhow::anyhow!("no active ws for {}", coin))

        }
    }
}