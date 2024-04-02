use std::collections::HashMap;

// TODO: Do we need all these?
pub enum TwitchIrcCommand {
    Privmsg {
        channel: String,
        sender: String,
        content: String,
    },
    Join {
        channel: String,
    },
    Part {
        channel: String,
    },
    Pass {
        token: String,
    },
    Nick {
        nick: String,
    },
    Ping {
        content: String,
    },
    Pong {
        content: String,
    },
    Numeric {
        command: u16,
        params: Vec<String>,
    },
}

pub struct RawIrcMessage {
    raw_tags: Option<String>,
    raw_origin: Option<String>,
    raw_command: String,
    params: Vec<String>,
}

#[derive(Debug)]
pub enum RawIrcParseError {
    BadSpaces,
    NoParams,
}

// TODO: Improve string cloning

impl TryFrom<String> for RawIrcMessage {
    type Error = RawIrcParseError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        let mut blocks = value.split(" ").map(|s| s.to_owned()).peekable();

        // Get tags, origin, command as owned strings
        let raw_tags = blocks
            .next_if(|b| b.starts_with("@"))
            .and_then(|b| b.strip_prefix("@").map(|b| b.to_owned()));
        let raw_origin = blocks
            .next_if(|b| b.starts_with(":"))
            .and_then(|b| b.strip_prefix(":").map(|b| b.to_owned()));
        let raw_command = blocks
            .next_if(|b| !b.is_empty())
            .ok_or(RawIrcParseError::BadSpaces)?;

        // Get params as owned strings, stripping the trailing param of ":" if any
        let mut params: Vec<String> = Vec::new();
        while let Some(param) = blocks.next_if(|b| !b.is_empty()) {
            if let Some(head) = param.strip_prefix(":") {
                let mut trailing = head.to_owned();
                if let Some(_) = blocks.peek() {
                    trailing.push(' ');
                    trailing.push_str(&blocks.collect::<Vec<String>>().as_slice().join(" "));
                }
                params.push(trailing);
                break;
            } else {
                params.push(param.to_owned());
            }
        }

        if params.is_empty() {
            Err(RawIrcParseError::NoParams)
        } else {
            Ok(RawIrcMessage {
                raw_tags,
                raw_origin,
                raw_command,
                params,
            })
        }
    }
}

pub struct TwitchIrcMessage {
    pub command: TwitchIrcCommand,
    tags: Option<HashMap<String, String>>,
}

#[derive(Debug)]
pub enum TwitchIrcParseError {
    BadOrigin,
    BadSenderInOrigin,
    MismatchedSenderInOrigin,
    BadCommand,
    BadParams,
    MissingSender,
}

impl TryFrom<RawIrcMessage> for TwitchIrcMessage {
    type Error = TwitchIrcParseError;

    fn try_from(value: RawIrcMessage) -> Result<Self, Self::Error> {
        // TODO: parse tags into hashmap from raw_tags
        let tags: Option<HashMap<String, String>> = None;

        let sender: Option<String> = value
            .raw_origin
            .map(|b| {
                b.strip_suffix("tmi.twitch.tv")
                    .map(|s| s.to_owned())
                    .ok_or(TwitchIrcParseError::BadOrigin)
            })
            .transpose()?
            .filter(|b| !b.is_empty())
            .map(|prefix| {
                // If prefix is not empty, then it should be of one of two forms:
                // - "nick."
                // - "nick!nick@nick."
                // TODO: Parse the second form correctly.
                let (nick, remainder) = prefix
                    .split_once("!")
                    .ok_or(TwitchIrcParseError::BadSenderInOrigin)?;
                let (username, remainder) = remainder
                    .split_once("@")
                    .ok_or(TwitchIrcParseError::BadSenderInOrigin)?;
                let (hostname_prefix, remainder) = remainder
                    .split_once(".")
                    .ok_or(TwitchIrcParseError::BadSenderInOrigin)?;
                if remainder.is_empty() && nick == username && username == hostname_prefix {
                    Ok(username.to_owned())
                } else {
                    Err(TwitchIrcParseError::MismatchedSenderInOrigin)
                }
            })
            .transpose()?;

        // Here we validate params if needed for each command
        // TODO: add JOIN, because we receive one when we join a channel
        match value.raw_command.as_str() {
            "PRIVMSG" => {
                let sender = sender.ok_or(TwitchIrcParseError::MissingSender)?;
                let [channel, content]: [String; 2] = value
                    .params
                    .try_into()
                    .map_err(|_| TwitchIrcParseError::BadParams)?;
                let channel = channel
                    .strip_prefix("#")
                    .ok_or(TwitchIrcParseError::BadParams)?
                    .to_owned();
                Ok(TwitchIrcMessage {
                    command: TwitchIrcCommand::Privmsg {
                        channel,
                        sender,
                        content,
                    },
                    tags,
                })
            }
            "PING" => {
                let [content]: [String; 1] = value
                    .params
                    .try_into()
                    .map_err(|_| TwitchIrcParseError::BadParams)?;
                Ok(TwitchIrcMessage {
                    command: TwitchIrcCommand::Ping { content },
                    tags,
                })
            }
            "PONG" => {
                let [content]: [String; 1] = value
                    .params
                    .try_into()
                    .map_err(|_| TwitchIrcParseError::BadParams)?;
                Ok(TwitchIrcMessage {
                    command: TwitchIrcCommand::Pong { content },
                    tags,
                })
            }
            raw_command => {
                // Try to parse as numeric command
                if let Ok(num) = raw_command.parse::<u16>() {
                    Ok(TwitchIrcMessage {
                        command: TwitchIrcCommand::Numeric {
                            command: num,
                            params: value.params,
                        },
                        tags,
                    })
                } else {
                    Err(TwitchIrcParseError::BadCommand)
                }
            }
        }
    }
}

#[derive(Debug)]
pub struct StringifyNotImplementedError;

pub fn stringify_message(
    message: &TwitchIrcMessage,
) -> Result<String, StringifyNotImplementedError> {
    match &message.command {
        TwitchIrcCommand::Privmsg {
            channel,
            sender,
            content,
        } => Ok(format!("[#{}] {}: {}", channel, sender, content)),
        TwitchIrcCommand::Ping { content } => Ok(format!("[ping {}]", content)),
        _ => Err(StringifyNotImplementedError),
    }
}
