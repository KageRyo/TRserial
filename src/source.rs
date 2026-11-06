use crate::{parser, store::Store};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub source: String,
    pub id: Option<String>,
    pub text: String,
}
pub trait MessageSource {
    fn messages(&mut self) -> Result<Vec<Message>>;
}
pub struct ManualSource(pub String);
impl MessageSource for ManualSource {
    fn messages(&mut self) -> Result<Vec<Message>> {
        Ok(vec![Message {
            source: "manual".into(),
            id: None,
            text: std::mem::take(&mut self.0),
        }])
    }
}
/// JSON array of messages, usable without any network or Discord credentials.
pub struct MockSource(pub Vec<Message>);
impl MessageSource for MockSource {
    fn messages(&mut self) -> Result<Vec<Message>> {
        Ok(std::mem::take(&mut self.0))
    }
}
pub fn ingest(store: &mut Store, message: &Message) -> Result<usize> {
    ensure!(message.text.len() <= 1024 * 1024, "訊息超過 1 MiB");
    let parsed = parser::parse(&message.text);
    store.ingest(
        &message.source,
        message.id.as_deref(),
        &message.text,
        &parsed,
    )
}
pub fn ingest_source(store: &mut Store, source: &mut dyn MessageSource) -> Result<usize> {
    source
        .messages()?
        .iter()
        .try_fold(0, |n, m| Ok(n + ingest(store, m)?))
}
#[cfg(feature = "clipboard")]
pub struct ClipboardSource(arboard::Clipboard);
#[cfg(feature = "clipboard")]
impl ClipboardSource {
    pub fn new() -> Result<Self> {
        Ok(Self(arboard::Clipboard::new()?))
    }
}
#[cfg(feature = "clipboard")]
impl MessageSource for ClipboardSource {
    fn messages(&mut self) -> Result<Vec<Message>> {
        // Unrelated text never leaves this function or reaches storage/logging.
        let Ok(text) = self.0.get_text() else {
            return Ok(vec![]);
        };
        if text.len() > 1024 * 1024 || parser::parse(&text).is_empty() {
            return Ok(vec![]);
        }
        Ok(vec![Message {
            source: "clipboard".into(),
            id: None,
            text,
        }])
    }
}
