use anyhow::Result;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use tracing::{info, error, warn};

use crate::{
    database::Database,
    telegram::TelegramBot,
    hyperliquid::{WebSocketManager, WsTrade},
    config::Config,
};

#[derive(Debug, Clone)]
pub enum SubscriptionEvent {
    UserSubscribed { coin: String },
}

pub struct TradeCoordinator {
    database: Database,
    telegram_bot: TelegramBot,
    ws_manager: Arc<WebSocketManager>,
    config: Config,
    active_feeds: Arc<RwLock<HashMap<String, bool>>>,
    trade_tx: Arc<RwLock<Option<mpsc::UnboundedSender<WsTrade>>>>,
}

impl TradeCoordinator {
    pub fn new(
        database: Database,
        telegram_bot: TelegramBot,
        ws_manager: WebSocketManager,
        config: Config,
    ) -> (Self, mpsc::UnboundedSender<SubscriptionEvent>, mpsc::UnboundedReceiver<SubscriptionEvent>) {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        
        let coordinator = TradeCoordinator {
            database,
            telegram_bot,
            ws_manager: Arc::new(ws_manager),
            config,
            active_feeds: Arc::new(RwLock::new(HashMap::new())),
            trade_tx: Arc::new(RwLock::new(None)),
        };
        
        (coordinator, event_tx, event_rx)
    }

    pub async fn start(self, mut event_rx: mpsc::UnboundedReceiver<SubscriptionEvent>) -> Result<()> {
        let active_coins = self.database.get_active_coins().await?;

        let (trade_tx, mut trade_rx) = mpsc::unbounded_channel::<WsTrade>();
        
        {
            let mut sender_lock = self.trade_tx.write().await;
            *sender_lock = Some(trade_tx.clone());
        }

        for coin in &active_coins {
            self.start_websocket_for_coin(coin).await;
        }

        info!("coordinator listening...");
        loop {
            tokio::select! {
                Some(trade) = trade_rx.recv() => {
                    if let Err(e) = self.process_trade(trade).await {
                        error!("error processing trade: {}", e);
                    }
                }
                
                Some(event) = event_rx.recv() => {
                    if let Err(e) = self.handle_subscription_event(event).await {
                        error!("error handling subscription event: {}", e);
                    }
                }
                
                else => {
                    break;
                }
            }
        }

        Ok(())
    }

    async fn handle_subscription_event(&self, event: SubscriptionEvent) -> Result<()> {
        match event {
            SubscriptionEvent::UserSubscribed { coin } => {
                info!("handle user subscription to {}", coin);
                self.check_coin_subscription(&coin).await?;
            }
        }
        Ok(())
    }

    async fn process_trade(&self, trade: WsTrade) -> Result<()> {
        let notional_usd = trade.notional_usd()?;

        if notional_usd < self.config.defaults.min_trade_value_usd {
            return Ok(());
        }

        info!("processing large {} trade: ${:.2}", trade.coin, notional_usd);

        let subscribers = self.database.get_subscribers_for_coin(&trade.coin).await?;
        
        if subscribers.is_empty() {
            warn!("No subscribers for {}, stopping WebSocket", trade.coin);
            
            // close ws if no one subbed
            if let Err(e) = self.ws_manager.stop_trade_feed(&trade.coin).await {
                error!("could close ws for {}: {}", trade.coin, e);
            } else {
                let mut active_feeds = self.active_feeds.write().await;
                active_feeds.remove(&trade.coin.to_uppercase());
                info!("closed ws for {}", trade.coin);
            }
            
            return Ok(());
        }

        info!("sending {} trade notification to {} subscribers", trade.coin, subscribers.len());

        for subscriber in subscribers {
            let telegram_bot = self.telegram_bot.clone();
            let trade_clone = trade.clone();
            let notional_clone = notional_usd;

            tokio::spawn(async move {
                if let Err(e) = telegram_bot.send_trade_notification(
                    subscriber.telegram_chat_id,
                    &trade_clone.coin,
                    &trade_clone.side,
                    &trade_clone.px,
                    notional_clone,
                ).await {
                    error!("Failed to send notification to chat {}: {}", subscriber.telegram_chat_id, e);
                }
            });
        }

        Ok(())
    }

    async fn check_coin_subscription(&self, coin: &str) -> Result<()> {
        let coin_upper = coin.to_uppercase();
        
        {
            let active_feeds = self.active_feeds.read().await;
            if active_feeds.contains_key(&coin_upper) {
                info!("ws alr exists for {}", coin_upper);
                return Ok(());
            }
        }

        self.start_websocket_for_coin(&coin_upper).await;
        
        Ok(())
    }

    async fn start_websocket_for_coin(&self, coin: &str) {
        let coin_upper = coin.to_uppercase();
        
        let trade_tx = {
            let sender_lock = self.trade_tx.read().await;
            match sender_lock.as_ref() {
                Some(tx) => tx.clone(),
                None => {
                    error!("coordinator not up");
                    return;
                }
            }
        };

        match self.ws_manager.start_trade_feed(&coin_upper, trade_tx).await {
            Ok(_) => {
                let mut active_feeds = self.active_feeds.write().await;
                active_feeds.insert(coin_upper.clone(), true);
                info!("ws feed started for {}", coin_upper);
            }
            Err(e) => {
                error!("failed to start ws for {}: {}", coin_upper, e);
            }
        }
    }
}

impl Clone for TradeCoordinator {
    fn clone(&self) -> Self {
        TradeCoordinator {
            database: self.database.clone(),
            telegram_bot: self.telegram_bot.clone(),
            ws_manager: self.ws_manager.clone(),
            config: self.config.clone(),
            active_feeds: self.active_feeds.clone(),
            trade_tx: self.trade_tx.clone(),
        }
    }
}