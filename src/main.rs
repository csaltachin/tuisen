use std::sync::mpsc;
use std::time::Duration;
use std::{io, thread};

use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
    KeyModifiers,
};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};

use ratatui::prelude::{Backend, CrosstermBackend, Terminal};

mod client;
use client::TwitchClientConfig;

mod actions;
use actions::{TerminalAction, TwitchAction};

mod config;
use config::TwitchLogin;

mod app;
use app::{App, ChatItem, InputMode, ScrollState};

mod ui;
use ui::render_ui;

mod irc;

const DEFAULT_IRC_ADDR: &str = "irc.chat.twitch.tv:6667";
const DEFAULT_CHANNEL: &str = "forsen";

fn main() -> io::Result<()> {
    // Init buffer
    let mut stdout = io::stdout();
    enable_raw_mode()?;
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;

    // Init backend and TUI
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    let (init_width, init_height) = terminal.size().map(|rect| (rect.width, rect.height))?;

    // Setup panic hook for cleanup
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic| {
        cleanup_terminal().unwrap();
        default_hook(panic);
    }));

    // App goes here
    let app = App::init(init_width, init_height);
    let app_result = run_app(app, &mut terminal);

    // Clean up
    cleanup_terminal()?;

    app_result
}

fn run_app<B: Backend>(mut app: App, terminal: &mut Terminal<B>) -> io::Result<()> {
    let (twitch_action_tx, twitch_action_rx) = mpsc::channel::<TwitchAction>();
    let (terminal_action_tx, terminal_action_rx) = mpsc::channel::<TerminalAction>();

    // TODO: improve custom config handling
    // Also maybe move the AppConfig read to the App::init method? Or have the AppConfig live
    // inside the App struct.
    let client_config = match config::try_read_config() {
        Ok(app_config) => {
            terminal_action_tx
                .send(TerminalAction::PrintDebug(
                    "[client] Loaded configuration file.".to_owned(),
                ))
                .unwrap();
            TwitchClientConfig::new(
                DEFAULT_IRC_ADDR.to_owned(),
                app_config.login,
                app_config.channel.unwrap_or(DEFAULT_CHANNEL.to_owned()),
                app_config.bot_mode,
            )
        }
        Err(e) => {
            terminal_action_tx
                .send(TerminalAction::PrintDebug(format!(
                    "[client] Failed to read config file ({:?}). Loading default configuration.",
                    e
                )))
                .unwrap();
            TwitchClientConfig::new(
                DEFAULT_IRC_ADDR.to_owned(),
                TwitchLogin::Anonymous,
                DEFAULT_CHANNEL.to_owned(),
                config::BotMode::Off,
            )
        }
    };

    let _client_handle = thread::spawn(move || {
        let _ = client::connect_and_listen(client_config, twitch_action_rx, terminal_action_tx);
    });

    loop {
        // Draw UI
        terminal.draw(|f| render_ui(f, &mut app))?;

        // Poll terminal actions
        if let Ok(action) = terminal_action_rx.try_recv() {
            match action {
                TerminalAction::PrintDebug(debug_message) => {
                    app.push_to_chat(ChatItem::Debug {
                        content: debug_message,
                    });
                }
                TerminalAction::PrintPrivmsg {
                    channel,
                    username,
                    message,
                } => {
                    app.push_to_chat(ChatItem::Privmsg {
                        channel,
                        username,
                        message,
                    });
                }
                TerminalAction::PrintPing(content) => {
                    app.push_to_chat(ChatItem::Ping { content });
                }
            }
        }

        // Poll key events
        if let Ok(true) = event::poll(Duration::from_millis(30)) {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Release {
                    continue;
                }

                // Force quit
                if let KeyEvent {
                    code: KeyCode::Char('q'),
                    modifiers: KeyModifiers::CONTROL,
                    ..
                } = key
                {
                    break;
                }

                // TODO: make this look nicer, maybe yoinking some of the AppState updating to
                // methods on the AppState struct
                match app.input_mode {
                    InputMode::Normal => match key.code {
                        KeyCode::Char('q') => {
                            break;
                        }
                        KeyCode::Char('i') => {
                            app.input_mode = InputMode::Insert;
                        }
                        KeyCode::Up if app.scroll_active => {
                            let offset_limit = app.get_scroll_offset_limit();
                            app.scroll_state = match app.scroll_state {
                                ScrollState::Top => ScrollState::Top,
                                // Make sure we convert any Offset(offset_limit) into Top
                                ScrollState::Bottom => {
                                    if offset_limit == 1 {
                                        ScrollState::Top
                                    } else {
                                        ScrollState::Offset(1)
                                    }
                                }
                                ScrollState::Offset(n) if n + 1 == offset_limit => ScrollState::Top,
                                ScrollState::Offset(n) => ScrollState::Offset(n + 1),
                            };
                        }
                        KeyCode::Down if app.scroll_active => {
                            let offset_limit = app.get_scroll_offset_limit();
                            app.scroll_state = match app.scroll_state {
                                // Make sure we convert any Offset(0) into Bottom
                                ScrollState::Bottom | ScrollState::Offset(1) => ScrollState::Bottom,
                                ScrollState::Offset(n) => ScrollState::Offset(n - 1),
                                ScrollState::Top => {
                                    if offset_limit == 1 {
                                        ScrollState::Bottom
                                    } else {
                                        ScrollState::Offset(offset_limit - 1)
                                    }
                                }
                            };
                        }
                        KeyCode::Home if app.scroll_active => {
                            app.scroll_state = ScrollState::Top;
                        }
                        KeyCode::End if app.scroll_active => {
                            app.scroll_state = ScrollState::Bottom;
                        }
                        _ => {}
                    },
                    InputMode::Insert => match key.code {
                        KeyCode::Esc => {
                            app.input_mode = InputMode::Normal;
                        }
                        KeyCode::Backspace => {
                            if key.modifiers == KeyModifiers::ALT && app.input_field.len() > 0 {
                                app.input_field = app
                                    .input_field
                                    .trim_end()
                                    .rsplit_once(' ')
                                    .map_or(String::new(), |(m, _)| {
                                        let mut mo = m.to_owned();
                                        mo.push(' ');
                                        mo
                                    });
                            } else {
                                app.input_field.pop();
                            }
                        }
                        KeyCode::Enter => {
                            let trimmed = app.input_field.trim();
                            if trimmed.len() > 0 {
                                twitch_action_tx
                                    .send(TwitchAction::SendPrivmsg {
                                        message: trimmed.to_owned(),
                                    })
                                    .unwrap();
                                app.input_field.clear();
                            }
                        }
                        KeyCode::Char(c) => {
                            app.input_field.push(c);
                        }
                        _ => {}
                    },
                }
            }
        }
    }

    Ok(())
}

fn cleanup_terminal() -> io::Result<()> {
    disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture)?;

    Ok(())
}
