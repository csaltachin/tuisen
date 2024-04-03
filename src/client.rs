use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::net::TcpStream;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use crate::actions::{TerminalAction, TwitchAction};
use crate::config::{BotMode, TwitchLogin};
use crate::irc::{RawIrcMessage, TwitchIrcCommand, TwitchIrcMessage};

const LOGIN_TIMEOUT_SECONDS: u16 = 5;
const LOGIN_RETRY_SECONDS: u16 = 10;

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

enum TwitchLoginResult {
    Success,
    Fail,
    Timeout,
}

enum RawStreamAction {
    Receive(String),
    EndOfStream,
}

fn try_login(
    raw_rx: &Receiver<RawStreamAction>,
    writer: &mut BufWriter<TcpStream>,
    pass: &String,
    nick: &String,
) -> TwitchLoginResult {
    writer
        .write(format!("PASS {}\r\n", pass).as_bytes())
        .unwrap();
    writer
        .write(format!("NICK {}\r\n", nick).as_bytes())
        .unwrap();
    writer.flush().unwrap();

    if let Ok(raw_action) = raw_rx.recv_timeout(Duration::from_secs(LOGIN_TIMEOUT_SECONDS.into())) {
        match raw_action {
            RawStreamAction::Receive(raw) => RawIrcMessage::try_from(raw)
                .ok()
                .and_then(|irc_message| TwitchIrcMessage::try_from(irc_message).ok())
                .and_then(|twitch_irc_message| {
                    if let TwitchIrcCommand::Numeric { command: 1, .. } = twitch_irc_message.command
                    {
                        Some(TwitchLoginResult::Success)
                    } else {
                        None
                    }
                })
                .map_or_else(|| TwitchLoginResult::Fail, |res| res),
            RawStreamAction::EndOfStream => TwitchLoginResult::Fail,
        }
    } else {
        TwitchLoginResult::Timeout
    }
}

// TODO: Handle NOTICE, the missing numeric commands, and other commands if we add capabilities
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
        TwitchIrcCommand::Join { joiner, channel } => {
            terminal_action_tx
                .send(TerminalAction::PrintDebug(format!(
                    "[client] {} joined #{}.",
                    joiner, channel
                )))
                .unwrap();
        }
        TwitchIrcCommand::Numeric { ref command, .. } => match command {
            // Welcome messages after 001; we ignore them
            2 | 3 | 4 | 375 | 372 | 376 => {}
            // Join list messages; we ignore them for now. TODO: do we want to build a names list
            // with these?
            353 | 366 => {}
            // TODO: are there any others? Maybe 421 for unsupported IRC commands?
            _ => {
                terminal_action_tx
                    .send(TerminalAction::PrintDebug(format!("[raw] {}", default_raw)))
                    .unwrap();
            }
        },
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

    // Confirm successful auth (or retry) before sending JOIN
    loop {
        match try_login(&raw_rx, &mut writer, &pass, &nick) {
            TwitchLoginResult::Success => {
                break;
            }
            TwitchLoginResult::Fail => {
                terminal_action_tx
                    .send(TerminalAction::PrintDebug(format!(
                        "Auth failed. Retrying in {} seconds...",
                        LOGIN_RETRY_SECONDS
                    )))
                    .unwrap();
            }
            TwitchLoginResult::Timeout => {
                terminal_action_tx
                    .send(TerminalAction::PrintDebug(format!(
                        "Auth timed out. Retrying in {} seconds...",
                        LOGIN_RETRY_SECONDS
                    )))
                    .unwrap();
            }
        }
        thread::sleep(Duration::from_secs(LOGIN_RETRY_SECONDS.into()));
    }

    terminal_action_tx
        .send(TerminalAction::PrintDebug(format!(
            "[client] Auth successful! Connecting to channel #{}...",
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
