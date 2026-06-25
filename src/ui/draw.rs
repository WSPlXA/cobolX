use crate::ui::tui::{App, Sender};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
    Frame,
};

pub fn draw(f: &mut Frame, app: &mut App) {
    // vertical screen layout
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints(
            [
                Constraint::Length(8), // Spring Boot-style ASCII banner
                Constraint::Min(3),    // Chat Console log
                Constraint::Length(3), // Input prompt
                Constraint::Length(3), // Footer instructions
            ]
            .as_ref(),
        )
        .split(f.size());

    // 1. Spring Boot-Style ASCII Banner
    let banner_lines = vec![
        Line::from(Span::styled("  ____ ___  ____   ___  _     __  __ ", Style::default().fg(Color::Green))),
        Line::from(Span::styled(" / ___/ _ \\| __ ) / _ \\| |    \\ \\/ / ", Style::default().fg(Color::Green))),
        Line::from(Span::styled("| |  | | | |  _ \\| | | | |     \\  /  ", Style::default().fg(Color::Green))),
        Line::from(Span::styled("| |__| |_| | |_) | |_| | |___  /  \\  ", Style::default().fg(Color::Green))),
        Line::from(Span::styled(" \\____\\___/|____/ \\___/|_____|/_/\\_\\ ", Style::default().fg(Color::Green))),
        Line::from(vec![
            Span::styled(" :: COBOLX ::", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
            Span::styled("                  (v1.0.0)", Style::default().fg(Color::DarkGray)),
        ]),
    ];

    let header_block = Paragraph::new(banner_lines).block(
        Block::default()
            .borders(Borders::BOTTOM)
            .border_style(Style::default().fg(Color::DarkGray)),
    );
    f.render_widget(header_block, chunks[0]);

    if app.view_mode == crate::ui::tui::ViewMode::SandboxSelect {
        let current_dir = std::env::current_dir().unwrap_or_default();
        let parent_dir = current_dir.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| current_dir.clone());
        
        let current_border_color = if app.sandbox_active_option == 0 { Color::Green } else { Color::DarkGray };
        let parent_border_color = if app.sandbox_active_option == 1 { Color::Green } else { Color::DarkGray };

        let current_style = if app.sandbox_active_option == 0 {
            Style::default().fg(Color::LightGreen)
        } else {
            Style::default().fg(Color::Gray)
        };
        let parent_style = if app.sandbox_active_option == 1 {
            Style::default().fg(Color::LightGreen)
        } else {
            Style::default().fg(Color::Gray)
        };

        let sandbox_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(
                [
                    Constraint::Length(2), // Empty space
                    Constraint::Length(4), // Option 1
                    Constraint::Length(1), // Spacer
                    Constraint::Length(4), // Option 2
                    Constraint::Min(1),
                ]
                .as_ref(),
            )
            .split(chunks[1]);

        let opt1_text = format!(" [1] Current Directory (.)\n     Path: {}", current_dir.to_string_lossy());
        let opt1_widget = Paragraph::new(opt1_text)
            .style(current_style)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Option A ")
                    .border_style(Style::default().fg(current_border_color)),
            );
        f.render_widget(opt1_widget, sandbox_chunks[1]);

        let opt2_text = format!(" [2] Parent Directory (..)\n     Path: {}", parent_dir.to_string_lossy());
        let opt2_widget = Paragraph::new(opt2_text)
            .style(parent_style)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Option B ")
                    .border_style(Style::default().fg(parent_border_color)),
            );
        f.render_widget(opt2_widget, sandbox_chunks[3]);

        // Draw instructions
        let sandbox_help = " Tab / Up / Down: Toggle Sandbox Directory | Enter: Confirm Selection | Ctrl+C: Force Exit ";
        let sandbox_help_block = Paragraph::new(sandbox_help).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Sandbox Selector Instructions ")
                .border_style(Style::default().fg(Color::DarkGray)),
        );
        f.render_widget(sandbox_help_block, chunks[2]);

        let empty_block = Paragraph::new("").block(Block::default().borders(Borders::NONE));
        f.render_widget(empty_block, chunks[3]);

        return;
    }

    if app.view_mode == crate::ui::tui::ViewMode::Config {
        let ds_border_color = if app.config_active_field == 0 { Color::Green } else { Color::DarkGray };
        let glm_border_color = if app.config_active_field == 1 { Color::Green } else { Color::DarkGray };
        
        let save_style = if app.config_active_field == 2 {
            Style::default().fg(Color::Black).bg(Color::Green).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Green)
        };
        let cancel_style = if app.config_active_field == 3 {
            Style::default().fg(Color::Black).bg(Color::Green).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray)
        };

        let form_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(
                [
                    Constraint::Length(1), // empty
                    Constraint::Length(3), // DeepSeek Input
                    Constraint::Length(1), // empty
                    Constraint::Length(3), // GLM Input
                    Constraint::Length(2), // empty
                    Constraint::Length(3), // Buttons
                    Constraint::Min(1),
                ]
                .as_ref(),
            )
            .split(chunks[1]);

        let mut ds_text = app.config_deepseek_input.clone();
        if app.config_active_field == 0 {
            ds_text.push('█');
        }
        let ds_widget = Paragraph::new(ds_text).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" [1] DeepSeek API Key (deepseek-chat) ")
                .border_style(Style::default().fg(ds_border_color)),
        );
        f.render_widget(ds_widget, form_chunks[1]);

        let mut glm_text = app.config_glm_input.clone();
        if app.config_active_field == 1 {
            glm_text.push('█');
        }
        let glm_widget = Paragraph::new(glm_text).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" [2] GLM-4-Pro API Key (glm-4-pro) ")
                .border_style(Style::default().fg(glm_border_color)),
        );
        f.render_widget(glm_widget, form_chunks[3]);

        let button_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(
                [
                    Constraint::Percentage(30),
                    Constraint::Percentage(20), // Save
                    Constraint::Percentage(20), // Cancel
                    Constraint::Percentage(30),
                ]
                .as_ref(),
            )
            .split(form_chunks[5]);

        let save_p = Paragraph::new("      [ SAVE ]      ")
            .style(save_style)
            .block(Block::default().borders(Borders::NONE));
        f.render_widget(save_p, button_chunks[1]);

        let cancel_p = Paragraph::new("     [ CANCEL ]     ")
            .style(cancel_style)
            .block(Block::default().borders(Borders::NONE));
        f.render_widget(cancel_p, button_chunks[2]);

        let config_help = " Tab / Arrow Keys: Move Focus | Type: Input | Enter: Select/Save | Esc: Return to Chat (if keys configured) ";
        let config_help_block = Paragraph::new(config_help).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Config Instructions ")
                .border_style(Style::default().fg(Color::DarkGray)),
        );
        f.render_widget(config_help_block, chunks[2]);

        let empty_block = Paragraph::new("").block(Block::default().borders(Borders::NONE));
        f.render_widget(empty_block, chunks[3]);

        return;
    }

    // 2. Chat Log Panel (COBOLX Console)
    let mut display_lines = Vec::new();
    for msg in &app.messages {
        let (prefix, color) = match msg.sender {
            Sender::User => (" [User]   ", Color::Cyan),
            Sender::Cobolx => (" [COBOLX] ", Color::Green),
        };

        // Time indicator
        let time_span = Span::styled(
            format!(" ({})", msg.timestamp),
            Style::default().fg(Color::DarkGray),
        );

        // Sender tag
        let sender_span = Span::styled(
            prefix,
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        );

        // Separator
        let sep_span = Span::styled(" : ", Style::default().fg(Color::Gray));

        // Message text
        let text_style = match msg.sender {
            Sender::User => Style::default().fg(Color::White),
            Sender::Cobolx => Style::default().fg(Color::LightGreen),
        };
        
        let lines: Vec<&str> = msg.text.split('\n').collect();
        for (i, line_str) in lines.iter().enumerate() {
            if i == 0 {
                display_lines.push(Line::from(vec![
                    time_span.clone(),
                    sender_span.clone(),
                    sep_span.clone(),
                    Span::styled(*line_str, text_style),
                ]));
            } else {
                display_lines.push(Line::from(vec![
                    Span::styled(format!("               {}", line_str), text_style),
                ]));
            }
        }
        
        display_lines.push(Line::from(""));
    }

    let log_height = chunks[1].height as usize;
    let available_lines = if log_height > 2 { log_height - 2 } else { 0 };

    let console_width = if chunks[1].width > 2 { chunks[1].width - 2 } else { 1 } as usize;
    let mut total_wrapped_height = 0;
    for line in &display_lines {
        let content_len: usize = line.spans.iter().map(|s| s.content.chars().count()).sum();
        let line_height = if content_len == 0 {
            1
        } else {
            (content_len + console_width - 1) / console_width
        };
        total_wrapped_height += line_height;
    }

    let scroll_y = if total_wrapped_height > available_lines {
        (total_wrapped_height - available_lines) as u16
    } else {
        0
    };

    let console_title = match &app.active_agent {
        Some(agent) => format!(" COBOLX Console [Active: {}] ", agent),
        None => " COBOLX Console ".to_string(),
    };
    let border_color = if app.active_agent.is_some() { Color::Green } else { Color::DarkGray };

    let console_block = Paragraph::new(display_lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(console_title)
                .border_style(Style::default().fg(border_color)),
        )
        .wrap(ratatui::widgets::Wrap { trim: false })
        .scroll((scroll_y, 0));
    f.render_widget(console_block, chunks[1]);

    // 3. Input Prompt
    let mut input_text = app.input_text.clone();
    input_text.push('█'); // Block terminal cursor

    let input_block = Paragraph::new(input_text).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Type message to COBOLX ")
            .border_style(Style::default().fg(Color::Green)),
    );
    f.render_widget(input_block, chunks[2]);

    // 4. Footer Help
    let help_text = " Type /help for commands | Enter: Send | Esc (when input is empty): Exit TUI | Ctrl+C: Force Exit ";
    let help_block = Paragraph::new(help_text).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Instructions ")
            .border_style(Style::default().fg(Color::DarkGray)),
    );
    f.render_widget(help_block, chunks[3]);

    // 5. Autocomplete Dropdown popup (renders overlay above input prompt)
    let dropdown_type = app.get_dropdown_type();
    if app.show_dropdown && dropdown_type != crate::ui::tui::DropdownType::None {
        let (items, title) = match dropdown_type {
            crate::ui::tui::DropdownType::Commands => {
                let filtered = app.get_filtered_commands();
                let list_items: Vec<ListItem> = filtered
                    .iter()
                    .enumerate()
                    .map(|(idx, cmd)| {
                        let style = if idx == app.dropdown_index {
                            Style::default().fg(Color::Black).bg(Color::Green)
                        } else {
                            Style::default().fg(Color::Green)
                        };

                        let desc = match cmd.as_str() {
                            "/help" => "  Show help list",
                            "/clear" => " Clear console history",
                            "/about" => " About COBOLX",
                            "/model" => " Model routing override",
                            "/config" => " Open API configuration",
                            "/tokens" => " Show token consumption statistics",
                            "/init" => "   Scan sandbox directory for COBOL",
                            "/exit" => "  Exit TUI",
                            _ => "",
                        };

                        ListItem::new(Line::from(vec![
                            Span::styled(format!("{:<8}", cmd), style.add_modifier(Modifier::BOLD)),
                            Span::styled(desc, Style::default().fg(Color::DarkGray)),
                        ]))
                    })
                    .collect();
                (list_items, " Commands ")
            }
            crate::ui::tui::DropdownType::Files => {
                let filtered = app.get_filtered_files();
                let list_items: Vec<ListItem> = filtered
                    .iter()
                    .enumerate()
                    .take(10)
                    .map(|(idx, file)| {
                        let style = if idx == app.dropdown_index {
                            Style::default().fg(Color::Black).bg(Color::Green)
                        } else {
                            Style::default().fg(Color::Green)
                        };

                        ListItem::new(Line::from(vec![
                            Span::styled(file.clone(), style),
                        ]))
                    })
                    .collect();
                (list_items, " Files ")
            }
            _ => (Vec::new(), ""),
        };

        if !items.is_empty() {
            let popup_height = (items.len() + 2) as u16;
            let popup_width = 45;
            let popup_rect = Rect {
                x: chunks[2].x + 2,
                y: chunks[2].y.saturating_sub(popup_height),
                width: popup_width.min(chunks[2].width - 4),
                height: popup_height,
            };

            f.render_widget(Clear, popup_rect);

            let list = List::new(items).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(title)
                    .border_style(Style::default().fg(Color::Green)),
            );
            f.render_widget(list, popup_rect);
        }
    }
}
