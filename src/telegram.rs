use anyhow::Result;
use teloxide::{
    prelude::*,
    types::Me,
    utils::command::BotCommands,
};
use tracing::{info, error};
use tokio::sync::mpsc;
use crate::{database::Database, hyperliquid::HyperliquidClient, coordinator::SubscriptionEvent};

#[derive(BotCommands, Clone, Debug)]
#[command(rename_rule = "lowercase", description = "Hyperliquid Trade Alerts")]
pub enum Command {
    #[command(description = "Start the bot")]
    Start,
    
    #[command(description = "Subscribe to a coin (e.g. /subscribe ETH)")]
    Subscribe(String),
    
    #[command(description = "Unsubscribe from a coin (e.g. /unsubscribe ETH)")]
    Unsubscribe(String),
    
    #[command(description = "List your current subscriptions")]
    List,
    
    #[command(description = "Show help message")]
    Help,
}

#[derive(Clone)]
pub struct TelegramBot {
    bot: Bot,
    database: Database,
    hyperliquid_client: HyperliquidClient,
    event_sender: mpsc::UnboundedSender<SubscriptionEvent>,
}

impl TelegramBot {
    pub fn new(
        bot_token: String, 
        database: Database, 
        hyperliquid_client: HyperliquidClient,
        event_sender: mpsc::UnboundedSender<SubscriptionEvent>
    ) -> Self {
        let bot = Bot::new(bot_token);
        
        TelegramBot {
            bot,
            database,
            hyperliquid_client,
            event_sender,
        }
    }

    pub async fn start(&self) -> Result<()> {
        info!("Starting Telegram bot...");

        let me: Me = self.bot.get_me().await?;
        info!("Bot started: @{}", me.username());

        let bot_clone = self.bot.clone();
        let database_clone = self.database.clone();
        let hyperliquid_clone = self.hyperliquid_client.clone();
        let event_sender_clone = self.event_sender.clone();
        
        let handler = Update::filter_message()
            .filter_command::<Command>()
            .endpoint(move |bot: Bot, msg: Message, cmd: Command| {
                let database = database_clone.clone();
                let hyperliquid_client = hyperliquid_clone.clone();
                let event_sender = event_sender_clone.clone();
                async move {
                    handle_command(bot, msg, cmd, database, hyperliquid_client, event_sender).await
                }
            });

        Dispatcher::builder(bot_clone, handler)
            .enable_ctrlc_handler()
            .build()
            .dispatch()
            .await;

        Ok(())
    }

    pub async fn send_trade_notification(
        &self, 
        chat_id: i64, 
        coin: &str, 
        side: &str,
        price: &str,
        notional_usd: f64
    ) -> Result<()> {
        let side_text = if side == "B" { "BUY" } else { "SELL" };
        
        let message = format!(
            "{} Trade Alert\n\nAmount: ${:.2}\nType: {}\nPrice: ${}",
            coin,
            notional_usd,
            side_text,
            price
        );

        self.bot.send_message(ChatId(chat_id), message).await?;
        info!("Sent {} trade notification to chat {}", coin, chat_id);
        Ok(())
    }
}

async fn handle_command(
    bot: Bot, 
    msg: Message, 
    cmd: Command, 
    database: Database, 
    mut hyperliquid_client: HyperliquidClient,
    event_sender: mpsc::UnboundedSender<SubscriptionEvent>
) -> ResponseResult<()> {
    let chat_id = msg.chat.id.0;
    let user_id = msg.from().map(|user| user.id.0 as i64).unwrap_or(chat_id);
    
    info!("Received command from user {}: {:?}", user_id, cmd);

    match cmd {
        Command::Start => {
            //subscribe to btc for every new user
            match database.add_subscription(user_id, chat_id, "BTC").await {
                Ok(true) => {
                    let welcome_msg = "Welcome to Hyperliquid Trade Alerts!\n\nYou've been automatically subscribed to BTC trades.\n\nUse /subscribe <coin> to add more coins!";
                    bot.send_message(msg.chat.id, welcome_msg).await?;
                    info!("new user {} auto-subscribed to BTC", user_id);
                    
                    if let Err(e) = event_sender.send(SubscriptionEvent::UserSubscribed { 
                        coin: "BTC".to_string() 
                    }) {
                        error!("Failed to send BTC subscription event: {}", e);
                    }
                }
                Ok(false) => {
                    let welcome_msg = "Welcome back to Hyperliquid Trade Alerts!\n\nUse /subscribe <coin> to add more subscriptions!";
                    bot.send_message(msg.chat.id, welcome_msg).await?;
                }
                Err(e) => {
                    error!("Failed to auto-subscribe user {} to BTC: {}", user_id, e);
                    let welcome_msg = "Welcome to Hyperliquid Trade Alerts!\n\nUse /subscribe <coin> to get started!";
                    bot.send_message(msg.chat.id, welcome_msg).await?;
                }
            }
        }
        
        Command::Subscribe(coin_arg) => {
            if coin_arg.trim().is_empty() {
                bot.send_message(msg.chat.id, "Please specify a coin. Example: /subscribe ETH").await?;
                return Ok(());
            }

            let coin = coin_arg.trim().to_uppercase();
            
            // make sure coin exists
            match hyperliquid_client.coin_exists(&coin).await {
                Ok(true) => {
                    match database.add_subscription(user_id, chat_id, &coin).await {
                        Ok(true) => {
                            let success_msg = format!("Successfully subscribed to {} trades!", coin);
                            bot.send_message(msg.chat.id, success_msg).await?;
                            info!("user {} subscribed to {}", user_id, coin);
                            
                            //send to coordinator to open ws
                            if let Err(e) = event_sender.send(SubscriptionEvent::UserSubscribed { 
                                coin: coin.clone() 
                            }) {
                                error!("couldn't send subscription event for {}: {}", coin, e);
                            }
                        }
                        Ok(false) => {
                            let already_msg = format!("You're already subscribed to {} trades.", coin);
                            bot.send_message(msg.chat.id, already_msg).await?;
                        }
                        Err(e) => {
                            error!("db error for user {} subscribing to {}: {}", user_id, coin, e);
                            bot.send_message(msg.chat.id, "Sorry, there was an error. Please try again.").await?;
                        }
                    }
                }
                Ok(false) => {
                    let invalid_msg = format!("{} is not available on Hyperliquid. Use /help to see valid coins.", coin);
                    bot.send_message(msg.chat.id, invalid_msg).await?;
                }
                Err(e) => {
                    error!("couldn't validate {} for {}: {}", coin, user_id, e);
                    bot.send_message(msg.chat.id, "Sorry, there was an error validating the coin. Please try again.").await?;
                }
            }
        }

        Command::Unsubscribe(coin_arg) => {
            if coin_arg.trim().is_empty() {
                bot.send_message(msg.chat.id, "Please specify a coin. Example: /unsubscribe ETH").await?;
                return Ok(());
            }

            let coin = coin_arg.trim().to_uppercase();
            
            match database.remove_subscription(user_id, &coin).await {
                Ok(true) => {
                    let success_msg = format!("Successfully unsubscribed from {} trades.", coin);
                    bot.send_message(msg.chat.id, success_msg).await?;
                    info!("user {} unsubscribed from {}", user_id, coin);
                }
                Ok(false) => {
                    let not_subscribed_msg = format!("You weren't subscribed to {} trades.", coin);
                    bot.send_message(msg.chat.id, not_subscribed_msg).await?;
                }
                Err(e) => {
                    error!("db error for user {} unsubscribing from {}: {}", user_id, coin, e);
                    bot.send_message(msg.chat.id, "Sorry, there was an error. Please try again.").await?;
                }
            }
        }
        
        Command::List => {
            match database.get_user_subscriptions(user_id).await {
                Ok(coins) => {
                    if coins.is_empty() {
                        bot.send_message(
                            msg.chat.id, 
                            "You're not subscribed to any coins.\n\nUse /subscribe <coin> to get started!"
                        ).await?;
                    } else {
                        let coins_list = coins.join(", ");
                        let list_msg = format!("Your Subscriptions:\n\n{}", coins_list);
                        bot.send_message(msg.chat.id, list_msg).await?;
                    }
                }
                Err(e) => {
                    error!("db error getting subscriptions for user {}: {}", user_id, e);
                    bot.send_message(
                        msg.chat.id, 
                        "Sorry, there was an error retrieving your subscriptions. Please try again."
                    ).await?;
                }
            }
        }
        
        Command::Help => {
            let help_msg = "Hyperliquid Trade Alerts Help\n\n\
                I monitor large trades ($50,000+) on Hyperliquid and send you notifications.\n\n\
                Available Commands:\n\
                /start - Get started and subscribe to BTC\n\
                /subscribe <coin> - Subscribe to a coin (e.g. /subscribe ETH)\n\
                /unsubscribe <coin> - Unsubscribe from a coin\n\
                /list - Show your current subscriptions\n\
                /help - Show this help message\n\n\
                Examples:\n\
                /subscribe SOL - Get SOL trade alerts\n\
                /unsubscribe BTC - Stop BTC trade alerts\n\
                /list - See all your subscriptions";

            bot.send_message(msg.chat.id, help_msg).await?;
        }
    }

    Ok(())
}