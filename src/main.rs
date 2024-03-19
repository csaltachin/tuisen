use std::sync::mpsc;
use std::time::Duration;
use std::{io, thread};

use config::TwitchLogin;
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};

use ratatui::prelude::{Backend, Constraint, CrosstermBackend, Direction, Layout};
use ratatui::widgets::{Block, Borders, List, Paragraph};
use ratatui::{Frame, Terminal};

mod client;
use client::TwitchClientConfig;

mod actions;
use actions::{TerminalAction, TwitchAction};

mod config;

const DEFAULT_IRC_ADDR: &str = "irc.chat.twitch.tv:6667";
const DEFAULT_CHANNEL: &str = "forsen";

// TODO: Break off ui stuff into its own module

enum ScrollState {
    Bottom,
    Offset(usize),
}

struct App {
    chat_lines: Vec<String>,
    scroll_state: ScrollState,
    input_field: String,
}

impl App {
    fn init() -> Self {
        App {
            chat_lines: Vec::new(),
            scroll_state: ScrollState::Bottom,
            input_field: String::new(),
        }
    }
}

fn main() -> io::Result<()> {
    // Init buffer
    let mut stdout = io::stdout();
    enable_raw_mode()?;
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;

    // Init backend and TUI
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // App goes here
    let app = App::init();
    let app_result = run_app(app, &mut terminal);

    // Clean up buffer
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

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

        // Poll messages
        if let Ok(action) = terminal_action_rx.try_recv() {
            match action {
                TerminalAction::PrintDebug(debug_message) => {
                    app.chat_lines.push(debug_message);
                }
                TerminalAction::PrintPrivmsg {
                    channel,
                    username,
                    message,
                } => {
                    app.chat_lines
                        .push(format!("[#{}] {}: {}", channel, username, message));
                }
                TerminalAction::PrintPing(content) => {
                    app.chat_lines.push(format!("[ping {}]", content))
                }
            }
        }

        // Poll key events
        if let Ok(true) = event::poll(Duration::from_millis(30)) {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Release {
                    continue;
                }

                if key.code == KeyCode::Esc {
                    return Ok(());
                }
                if key.code == KeyCode::Backspace {
                    app.input_field.pop();
                }
                if key.code == KeyCode::Enter {
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
                if let KeyCode::Char(c) = key.code {
                    app.input_field.push(c);
                }
            }
        }
    }
}

fn render_ui(frame: &mut Frame, app: &mut App) {
    let main_areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),
            Constraint::Length(3), // TODO: make this grow as needed when we input a lot of text
        ])
        .split(frame.size());

    let chat_area = main_areas[0];

    // TODO: Add better scrolling and wrapping logic
    let chat_inner_height = (chat_area.height - 2) as usize;
    let chat_line_count = app.chat_lines.len();
    let chat_lines = match app.scroll_state {
        ScrollState::Bottom => {
            let lo = std::cmp::max(0, (chat_line_count as isize) - (chat_inner_height as isize))
                as usize;
            app.chat_lines.get(lo..).unwrap().to_vec()
        }
        ScrollState::Offset(offset) => {
            let lo = chat_line_count - chat_inner_height - offset;
            app.chat_lines
                .get(lo..lo + chat_inner_height)
                .unwrap()
                .to_vec()
        }
    };

    let chat_widget = List::new(chat_lines).block(Block::default().borders(Borders::ALL));
    frame.render_widget(chat_widget, chat_area);

    let input_area = main_areas[1];
    let input_widget =
        Paragraph::new(app.input_field.clone()).block(Block::default().borders(Borders::ALL));
    frame.render_widget(input_widget, input_area);

    let cursor_x = input_area.x + (app.input_field.len() as u16) + 1;
    let cursor_y = input_area.y + 1;
    frame.set_cursor(cursor_x, cursor_y);
}
