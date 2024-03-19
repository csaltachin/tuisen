use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::net::TcpStream;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use crate::actions::{TerminalAction, TwitchAction};
use crate::config::{BotMode, TwitchLogin};

// TODO: Break off irc stuff into its own module
// Also try to clone strings less often

// TODO: implement From<AppConfig> for this type, to make client initialization cleaner
pub struct TwitchClientConfig {
    irc_addr: String,
    login: TwitchLogin,
    channel: String,
    bot_mode: BotMode,
}

impl TwitchClientConfig {
    pub fn new(irc_addr: String, login: TwitchLogin, channel: String, bot_mode: BotMode) -> Self {
        TwitchClientConfig {
            irc_addr,
            login,
            channel,
            bot_mode,
        }
    }
}

pub struct TwitchClient {
    config: TwitchClientConfig,
    terminal_action_tx: mpsc::Sender<TerminalAction>,
    twitch_action_rx: mpsc::Receiver<TwitchAction>,
}

enum RawStreamAction {
    Receive(String),
    EndOfStream,
}

// TODO: Do we need all these?
enum TwitchIrcCommand {
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
}

struct RawIrcMessage {
    raw_tags: Option<String>,
    raw_origin: Option<String>,
    raw_command: String,
    params: Vec<String>,
}

#[derive(Debug)]
enum RawIrcParseError {
    BadSpaces,
    NoParams,
}

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

struct TwitchIrcMessage {
    command: TwitchIrcCommand,
    tags: Option<HashMap<String, String>>,
}

#[derive(Debug)]
enum TwitchIrcParseError {
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
                // If prefix is not empty, then it should be of the form "nick!name@host.", and it
                // should satisfy nick == name == host. Then the sender will be name
                // TODO: double check if this is correct... it fails to parse some JOIN commands
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
        // TODO: Add more commands here (e.g. numeric commands)
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
            _ => Err(TwitchIrcParseError::BadCommand),
        }
    }
}

#[derive(Debug)]
struct StringifyNotImplementedError;

fn stringify_message(message: &TwitchIrcMessage) -> Result<String, StringifyNotImplementedError> {
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

fn handle_message(
    writer: &mut BufWriter<TcpStream>,
    terminal_action_tx: &Sender<TerminalAction>,
    message: TwitchIrcMessage,
    bot_mode: &BotMode,
    default_raw: &String,
) -> io::Result<()> {
    match message.command {
        TwitchIrcCommand::Ping { ref content } => {
            // Print the ping
            terminal_action_tx
                .send(TerminalAction::PrintPing(content.to_string()))
                .unwrap();
            // Answer the ping
            writer.write(format!("PONG :{}\r\n", content).as_bytes())?;
            writer.flush()?;
        }
        TwitchIrcCommand::Privmsg {
            ref channel,
            ref sender,
            ref content,
        } => {
            // Print the privmsg
            terminal_action_tx
                .send(TerminalAction::PrintPrivmsg {
                    channel: channel.to_string(),
                    username: sender.to_string(),
                    message: content.to_string(),
                })
                .unwrap();

            // Check for bot commands
            // TODO: Document this, or remove it, or make it configurable somehow
            if let BotMode::WithPrefix(bot_command_prefix) = bot_mode {
                if let Some(raw_bot_command) = content.strip_prefix(bot_command_prefix) {
                    if let Some(echo_arg) = raw_bot_command.strip_prefix("echo ") {
                        // Echo some text
                        writer.write(
                            format!("PRIVMSG #{} :SingsMic {}\r\n", channel, echo_arg).as_bytes(),
                        )?;
                        writer.flush()?;
                    } else if raw_bot_command.starts_with("ping") {
                        // Answer a ping
                        writer.write(
                            format!("PRIVMSG #{} :pong FutureMan\r\n", channel).as_bytes(),
                        )?;
                        writer.flush()?;
                    } else if raw_bot_command == "raid" {
                        // Type +join, for DeepDarkDungeonBot raids
                        writer.write(format!("PRIVMSG #{} :+join\r\n", channel).as_bytes())?;
                        writer.flush()?;
                    }
                }
            }
        }
        _ => {
            terminal_action_tx
                .send(TerminalAction::PrintDebug(format!("[raw] {}", default_raw)))
                .unwrap();
        }
    };
    Ok(())
}

// Main entrypoint for client, should be called from a spawned thread
pub fn connect_and_listen(
    client_config: TwitchClientConfig,
    twitch_action_rx: Receiver<TwitchAction>,
    terminal_action_tx: Sender<TerminalAction>,
) -> io::Result<()> {
    terminal_action_tx
        .send(TerminalAction::PrintDebug(
            "[client] Attempting to connect to twitch...".to_string(),
        ))
        .unwrap();

    let stream = TcpStream::connect(client_config.irc_addr)?;

    terminal_action_tx
        .send(TerminalAction::PrintDebug(
            "[client] Connected to twitch!".to_string(),
        ))
        .unwrap();

    let reader = BufReader::new(stream.try_clone()?);
    let mut writer = BufWriter::new(stream);

    let (raw_tx, raw_rx) = mpsc::channel::<RawStreamAction>();
    let _reader_handle = thread::spawn(move || read_raw(reader, raw_tx));

    let (nick, pass) = if let TwitchLogin::Auth {
        ref username,
        ref token,
    } = client_config.login
    {
        terminal_action_tx
            .send(TerminalAction::PrintDebug(format!(
                "[client] Attempting to auth as \"{}\"...",
                username
            )))
            .unwrap();
        (username.clone(), format!("oauth:{}", token))
    } else {
        terminal_action_tx
            .send(TerminalAction::PrintDebug(
                "[client] Login info not specified -- will auth as anonymous user".to_owned(),
            ))
            .unwrap();
        ("justinfan1337".to_owned(), "forsenCD".to_owned())
    };

    writer.write(format!("PASS {}\r\n", pass).as_bytes())?;
    writer.write(format!("NICK {}\r\n", nick).as_bytes())?;
    writer.flush()?;

    // TODO: confirm successful auth before sending JOIN (i.e. await a 001 or NOTICE here)

    terminal_action_tx
        .send(TerminalAction::PrintDebug(format!(
            "[client] Connecting to channel #{}... (did auth succeed?)",
            client_config.channel
        )))
        .unwrap();

    writer.write(format!("JOIN #{}\r\n", client_config.channel).as_bytes())?;
    writer.flush()?;

    terminal_action_tx
        .send(TerminalAction::PrintDebug(
            "[client] Listening to messages now.".to_string(),
        ))
        .unwrap();

    loop {
        // Poll stream reader
        if let Ok(raw_action) = raw_rx.try_recv() {
            match raw_action {
                RawStreamAction::Receive(raw) => match RawIrcMessage::try_from(raw.clone()) {
                    Ok(irc_message) => {
                        match TwitchIrcMessage::try_from(irc_message) {
                            Ok(twitch_irc_message) => {
                                handle_message(
                                    &mut writer,
                                    &terminal_action_tx,
                                    twitch_irc_message,
                                    &client_config.bot_mode,
                                    &raw,
                                )?;
                            }
                            Err(twitch_irc_parse_error) => {
                                terminal_action_tx
                                        .send(TerminalAction::PrintDebug(format!(
                                            "[error] Encountered {:?} while parsing this message: \"{}\"",
                                            twitch_irc_parse_error, &raw
                                        )))
                                        .unwrap();
                            }
                        };
                    }
                    Err(irc_parse_error) => {
                        terminal_action_tx
                            .send(TerminalAction::PrintDebug(format!(
                                "[error] Encountered {:?} while parsing this message: \"{}\"",
                                irc_parse_error, &raw
                            )))
                            .unwrap();
                    }
                },
                RawStreamAction::EndOfStream => {
                    break;
                }
            }
        }

        // Poll twitch actions
        if let Ok(twitch_action) = twitch_action_rx.try_recv() {
            match twitch_action {
                TwitchAction::SendPrivmsg { message } => {
                    // Ignore this action if the current login is anonymous
                    if let TwitchLogin::Auth { ref username, .. } = client_config.login {
                        writer
                            .write(
                                format!("PRIVMSG #{} :{}\r\n", client_config.channel, message)
                                    .as_bytes(),
                            )
                            .unwrap();
                        writer.flush().unwrap();
                        terminal_action_tx
                            .send(TerminalAction::PrintPrivmsg {
                                channel: client_config.channel.clone(),
                                username: username.clone(),
                                message,
                            })
                            .unwrap();
                    }
                }
                _ => {}
            }
        }

        // Tick
        thread::sleep(Duration::from_millis(30))
    }

    terminal_action_tx
        .send(TerminalAction::PrintDebug(
            "[client] Connection closed.".to_string(),
        ))
        .unwrap();

    Ok(())
}

fn read_raw(mut reader: BufReader<TcpStream>, raw_tx: Sender<RawStreamAction>) {
    let mut buffer = String::new();

    while let Ok(msize) = reader.read_line(&mut buffer) {
        if msize == 0 {
            break;
        };
        let raw_message = buffer.replace("\r\n", "");
        raw_tx.send(RawStreamAction::Receive(raw_message)).unwrap();
        buffer.clear();
    }

    raw_tx.send(RawStreamAction::EndOfStream).unwrap();
}
