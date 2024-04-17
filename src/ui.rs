use ratatui::prelude::{Color, Constraint, Direction, Layout, Line, Span, Style, Stylize};
use ratatui::widgets::{Block, Borders, List, Paragraph};
use ratatui::Frame;

use crate::app::{App, InputMode, ScrollState, INSERT_LEN_WARN};

pub fn render_ui(frame: &mut Frame, app: &mut App) {
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
