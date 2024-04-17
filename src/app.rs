use textwrap::wrap;

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
    pub fn init(init_width: u16, init_height: u16) -> Self {
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
}
