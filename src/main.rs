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

use ratatui::prelude::{Backend, Color, Constraint, CrosstermBackend, Direction, Layout, Style};
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

enum ScrollState {
    Bottom,
    Offset(usize),
    Top,
}

enum InputMode {
    Normal,
    Insert,
}

struct App {
    chat_lines: Vec<String>,
    scroll_state: ScrollState,
    scroll_active: bool,
    input_field: String,
    input_mode: InputMode,
    chat_width: u16,
    chat_height: u16,
}

impl App {
    fn init(init_width: u16, init_height: u16) -> Self {
        // TODO: do we want to compute chat_width and chat_height via the render
        // layout/constraints? What we have here is correct but hardcoded
        App {
            chat_lines: Vec::new(),
            scroll_state: ScrollState::Bottom,
            scroll_active: false,
            input_field: String::new(),
            input_mode: InputMode::Normal,
            // Subtract 2 from the left/right borders
            chat_width: init_width.saturating_sub(2),
            // Subtract 2 for the top/bottom borders, and 3 for the initial input area height
            chat_height: init_height.saturating_sub(5),
        }
    }

    fn get_scroll_offset_limit(&self) -> usize {
        self.chat_lines
            .len()
            .saturating_sub(self.chat_height.into())
    }

    fn refresh_chat_size(&mut self, new_chat_width: u16, new_chat_height: u16) {
        if self.chat_width != new_chat_width {
            self.chat_width = new_chat_width;
        }
        // Update height and adjust scroll state
        if self.chat_height != new_chat_height {
            let old_height = self.chat_height.clone();
            let lines = self.chat_lines.len();
            self.chat_height = new_chat_height;
            self.scroll_state = match self.scroll_state {
                // If there is no overflow anymore, reset scroll state to the initial state (Bottom
                // with scroll inactive)
                _ if lines <= (new_chat_height as usize) => {
                    self.scroll_active = false;
                    ScrollState::Bottom
                }
                // If we were in Bottom or Top, stay there
                ScrollState::Bottom => ScrollState::Bottom,
                ScrollState::Top => ScrollState::Top,
                // Otherwise, we'll try to fix the topmost displayed line. That is, we'll try to
                // preserve the number of hidden lines above, which is (line count - offset -
                // chat height).
                // Basically, if chat height changes by delta, then offset must change by -delta.
                // Then the new scroll state will be Offset(new offset), unless the new offset is 0
                // or negative, in which case we just set it to Bottom.
                // Notice that, at this point, we can assume there is overflow (otherwise we would
                // have pattern matched earlier). So in the Bottom case, we don't need to disable
                // scrolling.
                ScrollState::Offset(n) => {
                    let old_lines_above = lines.saturating_sub(n + (old_height as usize));
                    let new_offset =
                        lines.saturating_sub(old_lines_above + (new_chat_height as usize));
                    if new_offset > 0 {
                        ScrollState::Offset(new_offset)
                    } else {
                        ScrollState::Bottom
                    }
                }
            }
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
    let (init_width, init_height) = terminal.size().map(|rect| (rect.width, rect.height))?;

    // App goes here
    let app = App::init(init_width, init_height);
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
                        _ => {}
                    },
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
    let chat_line_count = app.chat_lines.len();

    let chat_inner_width = (chat_area.width - 2) as usize;
    let chat_inner_height = (chat_area.height - 2) as usize;

    // If the current chat size doesn't match the size in our AppState, update it
    app.refresh_chat_size(chat_inner_width as u16, chat_inner_height as u16);

    // If scroll is not active yet, check for overflow
    if !app.scroll_active && chat_line_count > chat_inner_height {
        app.scroll_active = true;
    }

    let chat_lines = match app.scroll_state {
        ScrollState::Bottom => {
            let lo = chat_line_count.saturating_sub(chat_inner_height);
            app.chat_lines.get(lo..).unwrap().to_vec()
        }
        ScrollState::Offset(offset) => {
            // At this point, offset should be strictly smaller than (chat_line_count -
            // chat_inner_height). Otherwise, something went wrong and we panic
            let lo = chat_line_count - chat_inner_height - offset;
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
    let input_border_color = match app.input_mode {
        InputMode::Normal => Color::default(),
        InputMode::Insert => Color::LightBlue,
    };
    let input_widget = Paragraph::new(app.input_field.clone()).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(input_border_color)),
    );
    frame.render_widget(input_widget, input_area);

    if let InputMode::Insert = app.input_mode {
        let cursor_x = input_area.x + (app.input_field.len() as u16) + 1;
        let cursor_y = input_area.y + 1;
        frame.set_cursor(cursor_x, cursor_y);
    }
}
