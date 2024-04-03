use std::collections::HashMap;

// TODO: Do we need all these?
pub enum TwitchIrcCommand {
    Privmsg {
        channel: String,
        sender: String,
        content: String,
    },
    Join {
        joiner: String,
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
                // If prefix is not empty, then it should have the following consecutive components:
                // - "nick!" (optional)
                // - "username@" (optional)
                // - "hostname_prefix." (always)
                // If more than one of these are present, then the present components
                // (nick, username, hostname_prefix) should match. For instance, when receiving a
                // PRIVMSG from Twitch, they should all match the author's username.
                let remainder = &prefix;
                let (nick_opt, remainder) = remainder
                    .split_once("!")
                    .map(|t| (Some(t.0), t.1))
                    .unwrap_or((None, remainder));
                let (username_opt, remainder) = remainder
                    .split_once("@")
                    .map(|t| (Some(t.0), t.1))
                    .unwrap_or((None, remainder));
                let (hostname_prefix_opt, remainder) = remainder
                    .split_once(".")
                    .map(|t| (Some(t.0), t.1))
                    .unwrap_or((None, remainder));

                // Note that prefix ends with '.' iff hostname_prefix_opt holds Some(_) and
                // remainder is empty.
                match (nick_opt, username_opt, hostname_prefix_opt, remainder) {
                    (None, None, Some(hostname_prefix), "") => Ok(hostname_prefix.to_owned()),
                    (Some(nick), Some(username), Some(hostname_prefix), "")
                        if nick == username && nick == hostname_prefix =>
                    {
                        Ok(username.to_owned())
                    }
                    (Some(_), Some(_), Some(_), "") => {
                        Err(TwitchIrcParseError::MismatchedSenderInOrigin)
                    }
                    // TODO: Do we want to contemplate when exactly two of the three fields are
                    // present? As far as I can tell, no such case appears in the Twitch docs.
                    _ => Err(TwitchIrcParseError::BadSenderInOrigin),
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
            "JOIN" => {
                let [hash_channel]: [String; 1] = value
                    .params
                    .try_into()
                    .map_err(|_| TwitchIrcParseError::BadParams)?;
                let channel = hash_channel
                    .split_once('#')
                    .filter(|t| t.0.is_empty() && !t.1.is_empty())
                    .map(|t| t.1.to_owned())
                    .ok_or(TwitchIrcParseError::BadParams)?;
                let joiner = sender.ok_or(TwitchIrcParseError::MissingSender)?;
                Ok(TwitchIrcMessage {
                    command: TwitchIrcCommand::Join { joiner, channel },
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
