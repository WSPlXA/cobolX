use crate::agent::client::AgentRouter;
use crate::ui::draw;
use chrono::Local;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sender {
    User,
    Cobolx,
}

#[derive(Debug, Clone)]
pub struct Message {
    pub sender: Sender,
    pub text: String,
    pub timestamp: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoutingMode {
    Auto,
    ForceLight,
    ForceHeavy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    Chat,
    Config,
    SandboxSelect,
}

pub enum TaskUpdate {
    Routed(crate::agent::client::Route, &'static str),
    Delta(String, &'static str),
    Finished(Result<Option<crate::agent::client::Usage>, String>, &'static str),
}

pub struct App {
    pub messages: Vec<Message>,
    pub input_text: String,
    pub dropdown_index: usize,
    pub show_dropdown: bool,
    pub routing_mode: RoutingMode,
    pub router: Arc<AgentRouter>,
    pub view_mode: ViewMode,
    pub config_active_field: usize,
    pub config_deepseek_input: String,
    pub config_glm_input: String,
    pub last_model: Option<String>,
    pub last_prompt_tokens: u32,
    pub last_completion_tokens: u32,
use crate::agent::client::AgentRouter;
use crate::ui::draw;
use chrono::Local;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sender {
    User,
    Cobolx,
}

#[derive(Debug, Clone)]
pub struct Message {
    pub sender: Sender,
    pub text: String,
    pub timestamp: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoutingMode {
    Auto,
    ForceLight,
    ForceHeavy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    Chat,
    Config,
    SandboxSelect,
}

pub enum TaskUpdate {
    Routed(crate::agent::client::Route, &'static str),
    Delta(String, &'static str),
    Finished(Result<Option<crate::agent::client::Usage>, String>, &'static str),
}

pub struct App {
    pub messages: Vec<Message>,
    pub input_text: String,
    pub dropdown_index: usize,
    pub show_dropdown: bool,
    pub routing_mode: RoutingMode,
    pub router: Arc<AgentRouter>,
    pub view_mode: ViewMode,
    pub config_active_field: usize,
    pub config_deepseek_input: String,
    pub config_glm_input: String,
    pub last_model: Option<String>,
    pub last_prompt_tokens: u32,
    pub last_completion_tokens: u32,
    pub deepseek_prompt_tokens: u32,
    pub deepseek_completion_tokens: u32,
    pub glm_prompt_tokens: u32,
    pub glm_completion_tokens: u32,
    pub sandbox_active_option: usize,
    pub sandbox_path: Option<std::path::PathBuf>,
    pub discovered_files: Vec<crate::cobol::scanner::CobolFileEntry>,
    pub active_agent: Option<String>,
}

impl App {
    pub fn new() -> Self {
        let router = Arc::new(AgentRouter::new());
        let mut messages = vec![Message {
            sender: Sender::Cobolx,
            text: "Hello! Welcome to the COBOLX console. Type your message below and press Enter to interact.".to_string(),
            timestamp: Local::now().format("%H:%M:%S").to_string(),
        }];

        let (_, config_data) = crate::config::ConfigManager::load_or_create();
        let has_keys = router.has_credentials();
        
        let view_mode = if !has_keys {
            ViewMode::Config
        } else {
            ViewMode::SandboxSelect
        };

        if !has_keys {
            let path_msg = if let Some(ref path) = router.config_path {
                format!("Please configure your API keys in the configuration file:\n  {}\nOr input them below directly in this screen.", path)
            } else {
                "Please enter your API keys below.".to_string()
            };
            messages.push(Message {
                sender: Sender::Cobolx,
                text: format!("WARNING: No API credentials found!\n{}", path_msg),
                timestamp: Local::now().format("%H:%M:%S").to_string(),
            });
        }

        App {
            messages,
            input_text: String::new(),
            dropdown_index: 0,
            show_dropdown: false,
            routing_mode: RoutingMode::Auto,
            router,
            view_mode,
            config_active_field: 0,
            config_deepseek_input: config_data.deepseek_api_key,
            config_glm_input: config_data.glm_api_key,
            last_model: None,
            last_prompt_tokens: 0,
            last_completion_tokens: 0,
            deepseek_prompt_tokens: 0,
            deepseek_completion_tokens: 0,
            glm_prompt_tokens: 0,
            glm_completion_tokens: 0,
            sandbox_active_option: 0,
            sandbox_path: None,
            discovered_files: Vec::new(),
            active_agent: None,
        }
    }

    pub fn get_filtered_commands(&self) -> Vec<String> {
        let commands = vec![
            "/help".to_string(),
            "/clear".to_string(),
            "/about".to_string(),
            "/model".to_string(),
            "/config".to_string(),
            "/tokens".to_string(),
            "/init".to_string(),
            "/exit".to_string(),
        ];
        if !self.input_text.starts_with('/') {
            return Vec::new();
        }
        commands
            .into_iter()
            .filter(|c| c.starts_with(&self.input_text))
            .collect()
    }

    /// Submits active input. Returns (should_exit, is_command)
    pub fn submit_message(&mut self) -> (bool, bool) {
        let text = self.input_text.trim().to_string();
        if text.is_empty() {
            return (false, false);
        }

        // Add user message
        self.messages.push(Message {
            sender: Sender::User,
            text: text.clone(),
            timestamp: Local::now().format("%H:%M:%S").to_string(),
        });

        let mut should_exit = false;
        let mut is_command = false;

        // Parse command if it starts with '/'
        if text.starts_with('/') {
            is_command = true;
            let parts: Vec<&str> = text.split_whitespace().collect();
            if !parts.is_empty() {
                let cmd = parts[0][1..].to_lowercase();
                match cmd.as_str() {
                    "exit" | "quit" => {
                        should_exit = true;
                    }
                    "clear" => {
                        self.messages.clear();
                        self.messages.push(Message {
                            sender: Sender::Cobolx,
                            text: "Console history cleared. Ready for next prompt.".to_string(),
                            timestamp: Local::now().format("%H:%M:%S").to_string(),
                        });
                    }
                    "about" => {
                        self.messages.push(Message {
                            sender: Sender::Cobolx,
                            text: "COBOLX Console v1.0.0. A high-performance terminal chat interface designed for AI agents, styled after Spring Boot.".to_string(),
                            timestamp: Local::now().format("%H:%M:%S").to_string(),
                        });
                    }
                    "help" => {
                        let help_message = "Available Commands:\n\
                                            /help             - Show this help message\n\
                                            /clear            - Clear the console chat log\n\
                                            /about            - Display information about COBOLX\n\
                                            /model            - Show current model routing setting\n\
                                            /model auto       - Set routing to Auto (via Router Sub-Agent)\n\
                                            /model light      - Force routing to Lightweight Model (DeepSeek)\n\
                                            /model heavy      - Force routing to Heavy Model (GLM-4-Pro)\n\
                                            /config           - Open the interactive API Key Configuration Screen\n\
                                            /tokens           - Show model routing and token consumption statistics\n\
                                            /init             - Scan the sandbox directory for COBOL files\n\
                                            /exit             - Close the interactive console";
                        self.messages.push(Message {
                            sender: Sender::Cobolx,
                            text: help_message.to_string(),
                            timestamp: Local::now().format("%H:%M:%S").to_string(),
                        });
                    }
                    "init" => {
                        if let Some(ref path) = self.sandbox_path {
                            self.messages.push(Message {
                                sender: Sender::Cobolx,
                                text: format!("Scanning sandbox directory (recursive): {}", path.to_string_lossy()),
                                timestamp: Local::now().format("%H:%M:%S").to_string(),
                            });

                            match crate::memory::MemoryStore::open_or_create(path)
                                .and_then(|mut store| {
                                    crate::cobol::indexer::index_sandbox(path, &mut store)
                                        .map(|report| (report, store.db_path().to_path_buf()))
                                })
                            {
                                Ok((report, db_path)) => {
                                    self.discovered_files = report.files.clone();
                                    if self.discovered_files.is_empty() {
                                        self.messages.push(Message {
                                            sender: Sender::Cobolx,
                                            text: "No COBOL files found in the sandbox (supported: .cbl, .cob, .cpy, .coo).".to_string(),
                                            timestamp: Local::now().format("%H:%M:%S").to_string(),
                                        });
                                    } else {
                                        self.messages.push(Message {
                                            sender: Sender::Cobolx,
                                            text: report.to_message(&db_path),
                                            timestamp: Local::now().format("%H:%M:%S").to_string(),
                                        });
                                    }
                        } else {
                            msg.text.push_str(&delta);
                        }
                    }
                }
                TaskUpdate::Finished(res, model_used) => {
                    app.active_agent = None;
                    match res {
                        Ok(Some(usage)) => {
                            app.last_model = Some(model_used.to_string());
                            app.last_prompt_tokens = usage.prompt_tokens;
                            app.last_completion_tokens = usage.completion_tokens;
                            
                            if model_used.contains("DeepSeek") {
                                app.deepseek_prompt_tokens += usage.prompt_tokens;
                                app.deepseek_completion_tokens += usage.completion_tokens;
                            } else if model_used.contains("GLM") {
                                app.glm_prompt_tokens += usage.prompt_tokens;
                                app.glm_completion_tokens += usage.completion_tokens;
                            }
                        }
                        Ok(None) => {
                            app.last_model = Some(model_used.to_string());
                            app.last_prompt_tokens = 0;
                            app.last_completion_tokens = 0;
                        }
                        Err(err) => {
                            if let Some(msg) = app.messages.iter_mut().last() {
                                if msg.text.contains("Thinking...") {
                                    msg.text = format!("(Using {}) Error: {}", model_used, err);
                                } else {
                                    msg.text.push_str(&format!("\n[Error: {}]", err));
                                }
                            }
                        }
                    }
                }
            }
        }

        terminal.draw(|f| draw::draw(f, app))?;

        // Non-blocking poll for crossterm events
        if event::poll(std::time::Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                if key.code == KeyCode::Char('c') && key.modifiers.contains(event::KeyModifiers::CONTROL) {
                    return Ok(());
                }

                if app.view_mode == ViewMode::Config {
                    match key.code {
                        KeyCode::Esc => {
                            if app.router.has_credentials() {
                                app.view_mode = ViewMode::Chat;
                            }
                        }
                        KeyCode::Tab | KeyCode::Down => {
                            app.config_active_field = (app.config_active_field + 1) % 4;
                        }
                        KeyCode::Up => {
                            app.config_active_field = (app.config_active_field + 3) % 4;
                        }
                        KeyCode::Enter => {
                            match app.config_active_field {
                                0 => {
                                    app.config_active_field = 1;
                                }
                                1 => {
                                    app.config_active_field = 2;
                                }
                                2 => {
                                    let new_data = crate::config::ConfigData {
                                        deepseek_api_key: app.config_deepseek_input.trim().to_string(),
                                        glm_api_key: app.config_glm_input.trim().to_string(),
                                    };
                                    match crate::config::ConfigManager::save(&new_data) {
                                        Ok(_) => {
                                            app.router = Arc::new(AgentRouter::new());
                                            app.view_mode = ViewMode::SandboxSelect;
                                            app.messages.push(Message {
                                                sender: Sender::Cobolx,
                                                text: "Configuration successfully saved and reloaded!".to_string(),
                                                timestamp: Local::now().format("%H:%M:%S").to_string(),
                                            });
                                        }
                                        Err(e) => {
                                            app.messages.push(Message {
                                                sender: Sender::Cobolx,
                                                text: format!("Error saving configuration: {}", e),
                                                timestamp: Local::now().format("%H:%M:%S").to_string(),
                                            });
                                        }
                                    }
                                }
                                3 => {
                                    if app.router.has_credentials() {
                                        app.view_mode = ViewMode::Chat;
                                    } else {
                                        app.messages.push(Message {
                                            sender: Sender::Cobolx,
                                            text: "Cannot cancel configuration: No API credentials found. Please set at least one key to save.".to_string(),
                                            timestamp: Local::now().format("%H:%M:%S").to_string(),
                                        });
                                    }
                                }
                                _ => {}
                            }
                        }
                        KeyCode::Char(c) => {
                            if app.config_active_field == 0 {
                                app.config_deepseek_input.push(c);
                            } else if app.config_active_field == 1 {
                                app.config_glm_input.push(c);
                            }
                        }
                        KeyCode::Backspace => {
                            if app.config_active_field == 0 {
                                app.config_deepseek_input.pop();
                            } else if app.config_active_field == 1 {
                                app.config_glm_input.pop();
                            }
                        }
                        _ => {}
                    }
                } else if app.view_mode == ViewMode::SandboxSelect {
                    match key.code {
                        KeyCode::Tab | KeyCode::Down | KeyCode::Up => {
                            app.sandbox_active_option = 1 - app.sandbox_active_option;
                        }
                        KeyCode::Enter => {
                            let resolved = if app.sandbox_active_option == 0 {
                                std::env::current_dir().ok()
                            } else {
                                std::env::current_dir().ok().and_then(|p| p.parent().map(|parent| parent.to_path_buf()))
                            };
                            if let Some(path) = resolved {
                                app.sandbox_path = Some(path.clone());
                                app.view_mode = ViewMode::Chat;
                                app.messages.push(Message {
                                    sender: Sender::Cobolx,
                                    text: format!("Sandbox directory set to: {}\nType /init to scan COBOL files in the sandbox.", path.to_string_lossy()),
                                    timestamp: Local::now().format("%H:%M:%S").to_string(),
                                });
                            } else {
                                app.messages.push(Message {
                                    sender: Sender::Cobolx,
                                    text: "Failed to resolve sandbox path. Please select current directory.".to_string(),
                                    timestamp: Local::now().format("%H:%M:%S").to_string(),
                                });
                            }
                        }
                        _ => {}
                    }
                } else {
                    let filtered = app.get_filtered_commands();
                    let has_options = !filtered.is_empty();

                    match key.code {
                        KeyCode::Esc => {
                            if app.show_dropdown {
                                app.show_dropdown = false;
                            } else if app.input_text.is_empty() {
                                return Ok(());
                            } else {
                                app.input_text.clear();
                            }
                        }
                        KeyCode::Down | KeyCode::Tab => {
                            if app.show_dropdown && has_options {
                                app.dropdown_index = (app.dropdown_index + 1) % filtered.len();
                            }
                        }
                        KeyCode::Up => {
                            if app.show_dropdown && has_options {
                                app.dropdown_index = (app.dropdown_index + filtered.len() - 1) % filtered.len();
                            }
                        }
                        KeyCode::Enter => {
                            if app.show_dropdown && has_options {
                                app.input_text = filtered[app.dropdown_index].clone();
                                app.show_dropdown = false;
                                if trigger_chat_task(app, tx) {
                                    return Ok(());
                                }
                            } else {
                                if trigger_chat_task(app, tx) {
                                    return Ok(());
                                }
                            }
                        }
                        KeyCode::Char(c) => {
                            app.input_text.push(c);
                            if app.input_text.starts_with('/') {
                                app.show_dropdown = true;
                                app.dropdown_index = 0;
                            }
                        }
                        KeyCode::Backspace => {
                            app.input_text.pop();
                            if app.input_text.starts_with('/') {
                                app.show_dropdown = true;
                                app.dropdown_index = 0;
                            } else {
                                app.show_dropdown = false;
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }
}
