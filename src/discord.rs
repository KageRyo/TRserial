//! Optional official Bot adapter. Disabled configuration never reads a token or opens a connection.
use crate::{
    config::DiscordConfig,
    source::{self, Message},
    store::Store,
};
use anyhow::{Context as _, Result, ensure};
use serenity::{
    async_trait,
    builder::GetMessages,
    client::{Client, Context, EventHandler},
    model::{
        channel::Message as DiscordMessage,
        event::ResumedEvent,
        gateway::{GatewayIntents, Ready},
        id::{ChannelId, MessageId},
    },
};
use std::path::{Path, PathBuf};
use tokio::{sync::Mutex, task::JoinHandle};
use tokio_util::sync::CancellationToken;

struct Handler {
    channels: Vec<u64>,
    path: PathBuf,
    gate: Mutex<()>,
}
impl Handler {
    fn save(&self, message: &DiscordMessage) -> Result<()> {
        let mut store = Store::open(&self.path)?;
        let source = format!("discord:{}", message.channel_id.get());
        source::ingest(
            &mut store,
            &Message {
                source,
                id: Some(message.id.to_string()),
                text: message.content.clone(),
            },
        )?;
        Ok(())
    }
    async fn catch_up(&self, ctx: &Context) -> Result<()> {
        let _guard = self.gate.lock().await;
        for &channel in &self.channels {
            let key = format!("discord:{channel}");
            let store = Store::open(&self.path)?;
            let cursor = store.cursor(&key)?.and_then(|s| s.parse::<u64>().ok());
            // First activation imports only the latest page. Subsequent runs recover from the durable cursor.
            let mut after = cursor;
            loop {
                let mut request = GetMessages::new().limit(100);
                if let Some(id) = after {
                    request = request.after(MessageId::new(id));
                }
                let mut messages = tokio::time::timeout(
                    std::time::Duration::from_secs(30),
                    ChannelId::new(channel).messages(&ctx.http, request),
                )
                .await
                .context("Discord history timeout")??;
                messages.sort_by_key(|m| m.id);
                let count = messages.len();
                for m in messages {
                    self.save(&m)?;
                    after = Some(m.id.get());
                    store.set_cursor(&key, &m.id.to_string())?;
                }
                if cursor.is_none() || count < 100 {
                    break;
                }
            }
        }
        Ok(())
    }
}
#[async_trait]
impl EventHandler for Handler {
    async fn message(&self, ctx: Context, message: DiscordMessage) {
        if !self.channels.contains(&message.channel_id.get()) {
            return;
        }
        // Live arrivals never advance the backfill cursor; reconnect cannot skip missing earlier messages.
        let result = {
            let _guard = self.gate.lock().await;
            self.save(&message)
        };
        if result.is_err() {
            tracing::warn!("Discord 訊息匯入失敗，將於補抓時再試");
        }
        // Catch up immediately so cursor progress is durable even during long healthy sessions.
        if self.catch_up(&ctx).await.is_err() {
            tracing::warn!("Discord 歷史補抓失敗，將於重連時再試");
        }
    }
    async fn ready(&self, ctx: Context, _: Ready) {
        if self.catch_up(&ctx).await.is_err() {
            tracing::warn!("Discord 歷史補抓失敗；請檢查頻道權限");
        }
    }
    async fn resume(&self, ctx: Context, _: ResumedEvent) {
        if self.catch_up(&ctx).await.is_err() {
            tracing::warn!("Discord 重連補抓失敗；游標已保留");
        }
    }
}
pub async fn start(
    config: &DiscordConfig,
    dir: &Path,
    cancel: CancellationToken,
) -> Result<Option<JoinHandle<()>>> {
    if !config.enabled {
        return Ok(None);
    }
    ensure!(
        !config.channels.is_empty() && config.channels.iter().all(|c| *c != 0),
        "啟用 Discord 前請設定 channels"
    );
    let token = zeroize::Zeroizing::new(
        std::env::var("TRSERIAL_DISCORD_TOKEN")
            .context("請設定 TRSERIAL_DISCORD_TOKEN；勿將 Bot Token 寫入 TOML")?,
    );
    let intents = GatewayIntents::GUILD_MESSAGES | GatewayIntents::MESSAGE_CONTENT;
    let mut client = Client::builder(token.as_str(), intents)
        .event_handler(Handler {
            channels: config.channels.clone(),
            path: dir.join("history.sqlite"),
            gate: Mutex::new(()),
        })
        .await
        .map_err(|_| anyhow::anyhow!("Discord Bot 初始化失敗；請檢查 Token"))?;
    let manager = client.shard_manager.clone();
    Ok(Some(tokio::spawn(async move {
        tokio::select! {
            _=cancel.cancelled()=>{},
            result=client.start()=>{if result.is_err() {tracing::warn!("Discord 連線停止；本機工作仍可繼續");}}
        }
        manager.shutdown_all().await;
    })))
}
