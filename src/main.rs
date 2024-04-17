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

use ratatui::prelude::{
    Backend, Color, Constraint, CrosstermBackend, Direction, Layout, Line, Span, Style, Stylize,
};
use ratatui::widgets::{Block, Borders, List, Paragraph};
use ratatui::{Frame, Terminal};

use textwrap::wrap;

mod client;
use client::TwitchClientConfig;

mod actions;
use actions::{TerminalAction, TwitchAction};

mod config;
use config::TwitchLogin;

mod irc;

const DEFAULT_IRC_ADDR: &str = "irc.chat.twitch.tv:6667";
const DEFAULT_CHANNEL: &str = "forsen";
const INSERT_LEN_WARN: usize = 500;

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

impl InputMode {
    fn title_string(&self) -> String {
        match self {
            InputMode::Normal => "[ normal ]".to_owned(),
            InputMode::Insert => "[ insert ]".to_owned(),
        }
    }
}

enum ChatItem {
    Privmsg {
        channel: String,
        username: String,
        message: String,
    },
    Debug {
        content: String,
    },
    Ping {
        content: String,
    },
}

impl ChatItem {
    fn wrapped_lines(&self, width: usize) -> Vec<String> {
        let unwrapped = match self {
            ChatItem::Debug { content } => content.clone(),
            ChatItem::Ping { content } => format!("[ping {}]", &content),
            ChatItem::Privmsg {
                channel,
                username,
                message,
            } => format!("[#{}] {}: {}", channel, username, message),
        };
        wrap(&unwrapped, width)
            .into_iter()
            .map(|cow| cow.to_string())
            .collect()
    }
}

struct App {
    chat_items: Vec<ChatItem>,
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
            chat_items: Vec::new(),
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

    fn push_to_chat(&mut self, item: ChatItem) {
        // TODO: wrap message before pushing line(s), and adjust scroll state correctly instead of
        // always by 1
        let item_lines = item.wrapped_lines(self.chat_width.into());
        let item_line_count = item_lines.len();
        self.chat_lines.extend(item_lines);
        self.chat_items.push(item);
        if let ScrollState::Offset(n) = self.scroll_state {
            if n > 0 {
                self.scroll_state = ScrollState::Offset(n + item_line_count);
            }
        };
    }

    fn get_scroll_offset_limit(&self) -> usize {
        self.chat_lines
            .len()
            .saturating_sub(self.chat_height.into())
    }

    fn refresh_chat_size(&mut self, new_chat_width: u16, new_chat_height: u16) {
        if self.chat_width != new_chat_width {
            // TODO: re-wrap messages when we implement wrapping
            self.chat_width = new_chat_width;
        }
        // Update height and adjust scroll state
        if self.chat_height != new_chat_height {
            let height_delta = new_chat_height as i32 - self.chat_height as i32;
            self.chat_height = new_chat_height;

            self.scroll_state = match self.scroll_state {
                // If there is no overflow anymore, reset scroll state to the initial state (Bottom
                // with scroll inactive)
                _ if self.chat_lines.len() <= (new_chat_height as usize) => {
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
                // have pattern-matched earlier). So in the Bottom case, we don't need to disable
                // scrolling.
                ScrollState::Offset(n) => {
                    let new_offset = n as i32 - height_delta;
                    if new_offset > 0 {
                        ScrollState::Offset(new_offset as usize)
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
    let mut input_widget_block = Block::default()
        .borders(Borders::ALL)
        .title_top(Line::from(app.input_mode.title_string()).left_aligned());
    // Custom block styling per mode
    input_widget_block = match app.input_mode {
        InputMode::Insert => {
            let trim_len = app.input_field.trim_end().len();
            let char_count_color = if trim_len > INSERT_LEN_WARN {
                Color::LightRed
            } else {
                input_border_color
            };
            let char_count_line = Line::from(vec![
                Span::raw("[ "),
                Span::raw(format!("{}/500", trim_len)).fg(char_count_color),
                Span::raw(" ]"),
            ])
            .right_aligned();
            input_widget_block.title_top(char_count_line)
        }
        _ => input_widget_block,
    };
    // Set the default border color on top of the previous titles
    input_widget_block = input_widget_block.border_style(Style::default().fg(input_border_color));

    let input_widget = Paragraph::new(app.input_field.clone()).block(input_widget_block);
    frame.render_widget(input_widget, input_area);

    if let InputMode::Insert = app.input_mode {
        let cursor_x = input_area.x + (app.input_field.len() as u16) + 1;
        let cursor_y = input_area.y + 1;
        frame.set_cursor(cursor_x, cursor_y);
    }
}

fn cleanup_terminal() -> io::Result<()> {
    disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture)?;

    Ok(())
}
