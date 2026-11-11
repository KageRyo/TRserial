//! Optional official Bot adapter. Disabled configuration never reads a token or opens a connection.
use crate::{
    accounts::Accounts,
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
use tokio::{
    sync::{Mutex, watch},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

/// Connection state for a caller that shows it, such as the desktop app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Connecting,
    Connected,
    /// Connected, but channel history could not be read.
    Degraded(String),
    Stopped(String),
}

/// `TRSERIAL_DISCORD_TOKEN` wins for services and scripts; otherwise use the
/// token the desktop app saved in the OS credential store.
pub fn resolve_token(accounts: &Accounts) -> Result<Zeroizing<String>> {
    if let Ok(token) = std::env::var("TRSERIAL_DISCORD_TOKEN") {
        return Ok(Zeroizing::new(token));
    }
    accounts.discord_token()?.context(
        "請在桌面版設定 Discord Bot Token，或設定 TRSERIAL_DISCORD_TOKEN；勿將 Token 寫入 TOML",
    )
}

struct Handler {
    channels: Vec<u64>,
    path: PathBuf,
    gate: Mutex<()>,
    status: Option<watch::Sender<Status>>,
}
impl Handler {
    fn report(&self, status: Status) {
        if let Some(sender) = &self.status {
            sender.send_replace(status);
        }
    }
    async fn catch_up_and_report(&self, ctx: &Context) -> bool {
        let ok = self.catch_up(ctx).await.is_ok();
        self.report(if ok {
            Status::Connected
        } else {
            Status::Degraded("無法讀取頻道訊息，請確認頻道 ID 與 Bot 的讀取權限".into())
        });
        ok
    }
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
        if !self.catch_up_and_report(&ctx).await {
            tracing::warn!("Discord 歷史補抓失敗，將於重連時再試");
        }
    }
    async fn ready(&self, ctx: Context, _: Ready) {
        if !self.catch_up_and_report(&ctx).await {
            tracing::warn!("Discord 歷史補抓失敗；請檢查頻道權限");
        }
    }
    async fn resume(&self, ctx: Context, _: ResumedEvent) {
        if !self.catch_up_and_report(&ctx).await {
            tracing::warn!("Discord 重連補抓失敗；游標已保留");
        }
    }
}

fn stopped_reason(error: &serenity::Error) -> String {
    use serenity::gateway::GatewayError;
    match error {
        serenity::Error::Gateway(GatewayError::InvalidAuthentication) => {
            "Bot Token 無效，請重新設定".into()
        }
        serenity::Error::Gateway(
            GatewayError::DisallowedGatewayIntents | GatewayError::InvalidGatewayIntents,
        ) => "請在 Discord Developer Portal 開啟 Bot 的 Message Content Intent".into(),
        _ => "Discord 連線中斷，請檢查網路或 Bot 設定".into(),
    }
}
pub async fn start(
    config: &DiscordConfig,
    dir: &Path,
    token: Zeroizing<String>,
    status: Option<watch::Sender<Status>>,
    cancel: CancellationToken,
) -> Result<Option<JoinHandle<()>>> {
    if !config.enabled {
        return Ok(None);
    }
    ensure!(
        !config.channels.is_empty() && config.channels.iter().all(|c| *c != 0),
        "啟用 Discord 前請設定 channels"
    );
    let report = status.clone();
    let intents = GatewayIntents::GUILD_MESSAGES | GatewayIntents::MESSAGE_CONTENT;
    let mut client = Client::builder(token.as_str(), intents)
        .event_handler(Handler {
            channels: config.channels.clone(),
            path: dir.join("history.sqlite"),
            gate: Mutex::new(()),
            status,
        })
        .await
        .map_err(|_| anyhow::anyhow!("Discord Bot 初始化失敗；請檢查 Token"))?;
    let manager = client.shard_manager.clone();
    Ok(Some(tokio::spawn(async move {
        // A caller that cancels is replacing or closing the listener and needs no report.
        let reason = tokio::select! {
            _ = cancel.cancelled() => None,
            result = client.start() => {
                tracing::warn!("Discord 連線停止；本機工作仍可繼續");
                Some(result.err().map_or_else(|| "Discord 連線已結束".into(), |error| stopped_reason(&error)))
            }
        };
        manager.shutdown_all().await;
        if let (Some(sender), Some(reason)) = (report, reason) {
            sender.send_replace(Status::Stopped(reason));
        }
    })))
}
