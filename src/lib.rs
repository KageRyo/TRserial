pub mod accounts;
pub mod browser;
pub mod config;
#[cfg(feature = "discord")]
pub mod discord;
pub mod notifications;
pub mod parser;
pub mod service;
pub mod source;
pub mod store;
pub mod worker;

pub mod driver;

/// Keep dependency HTTP traces from exposing browser form payloads in either client.
pub fn init_logging() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("trserial=info")
        .with_target(false)
        .try_init();
}
