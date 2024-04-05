use std::sync::mpsc;
use std::time::Duration;
use std::{io, thread};

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
use config::TwitchLogin;

mod irc;

const DEFAULT_IRC_ADDR: &str = "irc.chat.twitch.tv:6667";
const DEFAULT_CHANNEL: &str = "forsen";

// TODO: Break off ui stuff into its own module

#[derive(Clone)]
enum ScrollState {
    Bottom,
    Offset(isize),
    Top,
}

impl ScrollState {
    fn clipped(&self, hi: usize) -> Self {
        let ihi = hi as isize;
        match self {
            // If offset is negative, then we were scrolling down from the top
            ScrollState::Offset(ref n) if *n < 0 => ScrollState::Offset(ihi + *n),
            // If offset goes over hi, then we were trying to scroll up. If hi is 0, then we should
            // stay at the bottom; otherwise we clip at the top
            ScrollState::Offset(ref n) if *n > ihi => {
                if ihi == 0 {
                    ScrollState::Bottom
                } else {
                    ScrollState::Top
                }
            }
            state => state.clone(),
        }
    }
}

struct App {
    chat_lines: Vec<String>,
    scroll_state: ScrollState,
    scroll_active: bool,
    input_field: String,
}

impl App {
    fn init() -> Self {
        App {
            chat_lines: Vec::new(),
            scroll_state: ScrollState::Bottom,
            scroll_active: false,
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

        // Poll terminal actions
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
                    if let ScrollState::Offset(n) = app.scroll_state {
                        if n > 0 {
                            // TODO: Adjust this by the correct message height when we start
                            // wrapping messages
                            app.scroll_state = ScrollState::Offset(n + 1);
                        }
                    };
                }
                TerminalAction::PrintPing(content) => {
                    app.chat_lines.push(format!("[ping {}]", content));
                    if let ScrollState::Offset(n) = app.scroll_state {
                        if n > 0 {
                            // This should always be one line unless the window is really narrow
                            app.scroll_state = ScrollState::Offset(n + 1);
                        }
                    };
                }
            }
        }

        // Poll key events
        if let Ok(true) = event::poll(Duration::from_millis(30)) {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Release {
                    continue;
                }

                // TODO: input mode, scrolling
                match key.code {
                    KeyCode::Esc => {
                        break;
                    }
                    KeyCode::Backspace => {
                        app.input_field.pop();
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
                    KeyCode::Up if app.scroll_active => {
                        app.scroll_state = match app.scroll_state {
                            ScrollState::Bottom => ScrollState::Offset(1),
                            ScrollState::Offset(n) => ScrollState::Offset(n + 1),
                            ScrollState::Top => ScrollState::Top,
                        };
                    }
                    KeyCode::Down if app.scroll_active => {
                        app.scroll_state = match app.scroll_state {
                            ScrollState::Bottom | ScrollState::Offset(1) => ScrollState::Bottom,
                            ScrollState::Offset(n) => ScrollState::Offset(n - 1),
                            ScrollState::Top => ScrollState::Offset(-1),
                        };
                    }
                    _ => {}
                }
            }
        }
    }

    Ok(())
}

// TODO: Add better scrolling and wrapping logic
fn render_ui(frame: &mut Frame, app: &mut App) {
    let main_areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),
            Constraint::Length(3), // TODO: make this grow as needed when we input a lot of text
        ])
        .split(frame.size());

    let chat_area = main_areas[0];

    let chat_inner_height = (chat_area.height - 2) as usize;
    let chat_line_count = app.chat_lines.len();

    // If scroll is not active yet, check for overflow
    if !app.scroll_active && chat_line_count > chat_inner_height {
        app.scroll_active = true;
    }

    // Trim scroll offset if necessary
    app.scroll_state = app
        .scroll_state
        .clipped(chat_line_count.saturating_sub(chat_inner_height));

    let chat_lines = match app.scroll_state {
        ScrollState::Bottom => {
            let lo = chat_line_count.saturating_sub(chat_inner_height);
            app.chat_lines.get(lo..).unwrap().to_vec()
        }
        ScrollState::Offset(offset) => {
            // At this point, scroll_state has already been trimmed, so offset should be between 0
            // and (chat_line_count - chat_inner_height) inclusive. Otherwise something went wrong
            // and we panic
            let uoffset: usize = offset.try_into().unwrap();
            let lo = chat_line_count - chat_inner_height - uoffset;
            app.chat_lines
                .get(lo..lo + chat_inner_height)
                .unwrap()
                .to_vec()
        }
        ScrollState::Top => app.chat_lines.get(..chat_inner_height).unwrap().to_vec(),
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
