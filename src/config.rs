use std::{fs::read_to_string, io, path::Path};

use toml::{self, Table, Value};

#[derive(Debug)]
pub enum ConfigReadError {
    FileNotFound,
    BadPermissions,
    InvalidEncoding,
    InvalidSyntax,
    OtherError,
}

pub enum TwitchLogin {
    Anonymous,
    Auth { username: String, token: String },
}

pub enum BotMode {
    Off,
    WithPrefix(String),
}

// TODO: add more stuff, like UI options
pub struct AppConfig {
    pub login: TwitchLogin,
    pub channel: Option<String>,
    pub bot_mode: BotMode,
}

pub fn try_read_config() -> Result<AppConfig, ConfigReadError> {
    let config_path = Path::new("./tuisen.toml");

    let table = read_to_string(config_path)
        .map_err(|e| match e.kind() {
            io::ErrorKind::NotFound => ConfigReadError::FileNotFound,
            io::ErrorKind::InvalidData => ConfigReadError::InvalidEncoding,
            io::ErrorKind::PermissionDenied => ConfigReadError::BadPermissions,
            _ => ConfigReadError::OtherError,
        })?
        .parse::<Table>()
        .map_err(|_| ConfigReadError::InvalidSyntax)?;

    let login = match (table.get("username"), table.get("token")) {
        (Some(Value::String(username)), Some(Value::String(token))) => TwitchLogin::Auth {
            username: username.to_owned(),
            token: token.to_owned(),
        },
        _ => TwitchLogin::Anonymous,
    };

    let channel = match table.get("channel") {
        Some(Value::String(ref channel_name)) => Some(channel_name.to_owned()),
        _ => None,
    };

    let bot_mode = match table.get("bot_prefix") {
        Some(Value::String(ref prefix)) => BotMode::WithPrefix(prefix.to_owned()),
        _ => BotMode::Off,
    };

    Ok(AppConfig {
        login,
        channel,
        bot_mode,
    })
}
