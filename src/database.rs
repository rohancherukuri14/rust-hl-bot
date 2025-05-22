use anyhow::Result;
use sqlx::{PgPool, Row};
use tracing::{info, error};
use thiserror::Error;
use crate::config::DatabaseConfig;

#[derive(Error, Debug)]
pub enum DatabaseError {
    #[error("Database connection failed: {0}")]
    ConnectionError(#[from] sqlx::Error),
}

#[derive(Clone)]
pub struct Database {
    pool: PgPool, 
}

#[derive(Debug)]
pub struct UserSubscription {
    pub telegram_user_id: i64,
    pub telegram_chat_id: i64,
    pub coin: String,
}

impl Database {
    pub async fn new(config: &DatabaseConfig) -> Result<Self> {
        info!("connecting to db...");
        
        let pool = PgPool::connect(&config.url).await?;
        
        info!("connected to db");
        Ok(Database { pool })
    }

    pub async fn add_subscription(
        &self, 
        telegram_user_id: i64, 
        telegram_chat_id: i64, 
        coin: &str
    ) -> Result<bool> {
        let result = sqlx::query(
            r#"
            INSERT INTO user_subscriptions (telegram_user_id, telegram_chat_id, coin)
            VALUES ($1, $2, $3)
            ON CONFLICT (telegram_user_id, coin) DO NOTHING
            "#
        )
        .bind(telegram_user_id)
        .bind(telegram_chat_id)
        .bind(coin.to_uppercase())
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }

    pub async fn remove_subscription(&self, telegram_user_id: i64, coin: &str) -> Result<bool> {
        let result = sqlx::query("DELETE FROM user_subscriptions WHERE telegram_user_id = $1 AND coin = $2")
            .bind(telegram_user_id)
            .bind(coin.to_uppercase())
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    pub async fn get_user_subscriptions(&self, telegram_user_id: i64) -> Result<Vec<String>> {
        let rows = sqlx::query("SELECT coin FROM user_subscriptions WHERE telegram_user_id = $1 ORDER BY coin")
            .bind(telegram_user_id)
            .fetch_all(&self.pool)
            .await?;

        let coins = rows.into_iter().map(|row| row.get::<String, _>("coin")).collect();
        Ok(coins)
    }

    pub async fn get_subscribers_for_coin(&self, coin: &str) -> Result<Vec<UserSubscription>> {
        let rows = sqlx::query("SELECT telegram_user_id, telegram_chat_id, coin FROM user_subscriptions WHERE coin = $1")
            .bind(coin.to_uppercase())
            .fetch_all(&self.pool)
            .await?;

        let subscriptions = rows
            .into_iter()
            .map(|row| UserSubscription {
                telegram_user_id: row.get::<i64, _>("telegram_user_id"),
                telegram_chat_id: row.get::<i64, _>("telegram_chat_id"),
                coin: row.get::<String, _>("coin"),
            })
            .collect();

        Ok(subscriptions)
    }


    pub async fn get_active_coins(&self) -> Result<Vec<String>> {
        let rows = sqlx::query("SELECT DISTINCT coin FROM user_subscriptions ORDER BY coin")
            .fetch_all(&self.pool)
            .await?;

        let coins = rows.into_iter().map(|row| row.get::<String, _>("coin")).collect();
        Ok(coins)
    }
}

pub async fn init(config: &DatabaseConfig) -> Result<Database> {
    Database::new(config).await
}