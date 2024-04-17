use std::sync::mpsc::{Receiver, Sender};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use textwrap::wrap;

use crate::actions::{TerminalAction, TwitchAction};

pub const INSERT_LEN_WARN: usize = 500;

pub enum ScrollState {
    Bottom,
    Offset(usize),
    Top,
}

pub enum InputMode {
    Normal,
    Insert,
}

impl InputMode {
    pub fn title_string(&self) -> String {
        match self {
            InputMode::Normal => "[ normal ]".to_owned(),
            InputMode::Insert => "[ insert ]".to_owned(),
        }
    }
}

pub enum ChatItem {
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

pub struct App {
    pub terminal_action_rx: Receiver<TerminalAction>,
    pub twitch_action_tx: Sender<TwitchAction>,
    pub chat_items: Vec<ChatItem>,
    pub chat_lines: Vec<String>,
    pub scroll_state: ScrollState,
    pub scroll_active: bool,
    pub input_field: String,
    pub input_mode: InputMode,
    pub chat_width: u16,
    pub chat_height: u16,
}

impl App {
    pub fn init(
        init_width: u16,
        init_height: u16,
        terminal_action_rx: Receiver<TerminalAction>,
        twitch_action_tx: Sender<TwitchAction>,
    ) -> Self {
        // TODO: do we want to compute chat_width and chat_height via the render
        // layout/constraints? What we have here is correct but hardcoded
        App {
            terminal_action_rx,
            twitch_action_tx,
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

    pub fn push_to_chat(&mut self, item: ChatItem) {
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

    pub fn get_scroll_offset_limit(&self) -> usize {
        self.chat_lines
            .len()
            .saturating_sub(self.chat_height.into())
    }

    pub fn refresh_chat_size(&mut self, new_chat_width: u16, new_chat_height: u16) {
        if self.chat_width != new_chat_width {
            self.chat_width = new_chat_width;
            self.rewrap_lines();
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

    pub fn rewrap_lines(&mut self) {
        let old_line_count = self.chat_lines.len();
        self.chat_lines = self
            .chat_items
            .iter()
            .map(|item| item.wrapped_lines(self.chat_width.into()))
            .flatten()
            .collect();
        let line_count_delta = self.chat_lines.len() as i32 - old_line_count as i32;

        // TODO: This is almost identical to the scroll adjusting in refresh_chat_size. Maybe we
        // can yoink it out into its own method, taking an offset delta as a parameter?
        self.scroll_state = match self.scroll_state {
            _ if self.chat_lines.len() <= self.chat_height.into() => {
                self.scroll_active = false;
                ScrollState::Bottom
            }
            ScrollState::Bottom => ScrollState::Bottom,
            ScrollState::Top => ScrollState::Top,
            ScrollState::Offset(n) => {
                let new_offset = n as i32 + line_count_delta;
                if new_offset > 0 {
                    ScrollState::Offset(new_offset as usize)
                } else {
                    ScrollState::Bottom
                }
            }
        }
    }

    pub fn try_recv_terminal_action(&mut self) {
        if let Ok(action) = self.terminal_action_rx.try_recv() {
            match action {
                TerminalAction::PrintDebug(debug_message) => {
                    self.push_to_chat(ChatItem::Debug {
                        content: debug_message,
                    });
                }
                TerminalAction::PrintPrivmsg {
                    channel,
                    username,
                    message,
                } => {
                    self.push_to_chat(ChatItem::Privmsg {
                        channel,
                        username,
                        message,
                    });
                }
                TerminalAction::PrintPing(content) => {
                    self.push_to_chat(ChatItem::Ping { content });
                }
            }
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        match self.input_mode {
            InputMode::Normal => match key.code {
                KeyCode::Char('q') => true,
                KeyCode::Char('i') => {
                    self.input_mode = InputMode::Insert;
                    false
                }
                KeyCode::Up if self.scroll_active => {
                    let offset_limit = self.get_scroll_offset_limit();
                    self.scroll_state = match self.scroll_state {
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
                    false
                }
                KeyCode::Down if self.scroll_active => {
                    let offset_limit = self.get_scroll_offset_limit();
                    self.scroll_state = match self.scroll_state {
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
                    false
                }
                KeyCode::Home if self.scroll_active => {
                    self.scroll_state = ScrollState::Top;
                    false
                }
                KeyCode::End if self.scroll_active => {
                    self.scroll_state = ScrollState::Bottom;
                    false
                }
                _ => false,
            },
            InputMode::Insert => match key.code {
                KeyCode::Esc => {
                    self.input_mode = InputMode::Normal;
                    false
                }
                KeyCode::Backspace => {
                    if key.modifiers == KeyModifiers::ALT && self.input_field.len() > 0 {
                        self.input_field = self.input_field.trim_end().rsplit_once(' ').map_or(
                            String::new(),
                            |(m, _)| {
                                let mut mo = m.to_owned();
                                mo.push(' ');
                                mo
                            },
                        );
                    } else {
                        self.input_field.pop();
                    };
                    false
                }
                KeyCode::Enter => {
                    let trimmed = self.input_field.trim();
                    if trimmed.len() > 0 {
                        self.twitch_action_tx
                            .send(TwitchAction::SendPrivmsg {
                                message: trimmed.to_owned(),
                            })
                            .unwrap();
                        self.input_field.clear();
                    };
                    false
                }
                KeyCode::Char(c) => {
                    self.input_field.push(c);
                    false
                }
                _ => false,
            },
        }
    }
}
