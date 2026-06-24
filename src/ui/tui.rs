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

                            match crate::cobol::scanner::scan_sandbox(path) {
                                Ok(entries) => {
                                    self.discovered_files = entries;
                                    if self.discovered_files.is_empty() {
                                        self.messages.push(Message {
                                            sender: Sender::Cobolx,
                                            text: "No COBOL files found in the sandbox (supported: .cbl, .cob, .cpy, .coo).\nSkips: .git, target, node_modules, vendor, build, hidden dirs.".to_string(),
                                            timestamp: Local::now().format("%H:%M:%S").to_string(),
                                        });
                                    } else {
                                        use crate::cobol::scanner::CobolFileType;
                                        let sources: Vec<_> = self.discovered_files.iter()
                                            .filter(|f| f.file_type == CobolFileType::Source)
                                            .collect();
                                        let copybooks: Vec<_> = self.discovered_files.iter()
                                            .filter(|f| f.file_type == CobolFileType::Copybook)
                                            .collect();

                                        let mut report = format!(
                                            "Found {} COBOL file(s): {} source(s), {} copybook(s)\n",
                                            self.discovered_files.len(),
                                            sources.len(),
                                            copybooks.len(),
                                        );

                                        if !sources.is_empty() {
                                            report.push_str("\n  Sources:");
                                            for f in &sources {
                                                report.push_str(&format!("\n    - {} ({} bytes)", f.path.to_string_lossy(), f.size_bytes));
                                            }
                                        }
                                        if !copybooks.is_empty() {
                                            report.push_str("\n  Copybooks:");
                                            for f in &copybooks {
                                                report.push_str(&format!("\n    - {} ({} bytes)", f.path.to_string_lossy(), f.size_bytes));
                                            }
                                        }

                                        self.messages.push(Message {
                                            sender: Sender::Cobolx,
                                            text: report,
                                            timestamp: Local::now().format("%H:%M:%S").to_string(),
                                        });
                                    }
                                }
                                Err(e) => {
                                    self.messages.push(Message {
                                        sender: Sender::Cobolx,
                                        text: format!("Failed to scan sandbox directory: {}", e),
                                        timestamp: Local::now().format("%H:%M:%S").to_string(),
                                    });
                                }
                            }
                        } else {
                            self.messages.push(Message {
                                sender: Sender::Cobolx,
                                text: "Error: No sandbox path configured. Please restart or select a sandbox directory first.".to_string(),
                                timestamp: Local::now().format("%H:%M:%S").to_string(),
                            });
                        }
                    }
                    "tokens" | "usage" => {
                        let model_str = self.last_model.clone().unwrap_or_else(|| "None (No queries sent yet)".to_string());
                        let last_total = self.last_prompt_tokens + self.last_completion_tokens;
                        let ds_total = self.deepseek_prompt_tokens + self.deepseek_completion_tokens;
                        let glm_total = self.glm_prompt_tokens + self.glm_completion_tokens;
                        let grand_total = ds_total + glm_total;
                        
                        let usage_message = format!(
                            "Model Routing & Token Metrics:\n\
                             ---------------------------------\n\
                             Last Routed Model      : {}\n\
                             Last Prompt Tokens     : {}\n\
                             Last Completion Tokens : {}\n\
                             Last Total Tokens      : {}\n\
                             \n\
                             Session Totals by Model:\n\
                             ---------------------------------\n\
                             [ DeepSeek ]\n\
                             Prompt Tokens          : {}\n\
                             Completion Tokens      : {}\n\
                             Total DeepSeek Tokens  : {}\n\
                             \n\
                             [ GLM-4-Pro ]\n\
                             Prompt Tokens          : {}\n\
                             Completion Tokens      : {}\n\
                             Total GLM-4-Pro Tokens : {}\n\
                             \n\
                             Grand Total Session    : {}",
                            model_str,
                            self.last_prompt_tokens,
                            self.last_completion_tokens,
                            last_total,
                            self.deepseek_prompt_tokens,
                            self.deepseek_completion_tokens,
                            ds_total,
                            self.glm_prompt_tokens,
                            self.glm_completion_tokens,
                            glm_total,
                            grand_total
                        );
                        self.messages.push(Message {
                            sender: Sender::Cobolx,
                            text: usage_message,
                            timestamp: Local::now().format("%H:%M:%S").to_string(),
                        });
                    }
                    "config" | "settings" => {
                        let (_, config_data) = crate::config::ConfigManager::load_or_create();
                        self.config_deepseek_input = config_data.deepseek_api_key;
                        self.config_glm_input = config_data.glm_api_key;
                        self.config_active_field = 0;
                        self.view_mode = ViewMode::Config;
                        self.messages.push(Message {
                            sender: Sender::Cobolx,
                            text: "Opening configuration screen... Use Tab/arrows to navigate, type to enter keys, and select Save.".to_string(),
                            timestamp: Local::now().format("%H:%M:%S").to_string(),
                        });
                    }
                    "model" => {
                        if parts.len() > 1 {
                            let arg = parts[1].to_lowercase();
                            match arg.as_str() {
                                "auto" => {
                                    self.routing_mode = RoutingMode::Auto;
                                    self.messages.push(Message {
                                        sender: Sender::Cobolx,
                                        text: "Model routing set to Auto. Router Sub-Agent will classify tasks dynamically.".to_string(),
                                        timestamp: Local::now().format("%H:%M:%S").to_string(),
                                    });
                                }
                                "light" => {
                                    self.routing_mode = RoutingMode::ForceLight;
                                    self.messages.push(Message {
                                        sender: Sender::Cobolx,
                                        text: "Model routing set to ForceLight. All queries sent to DeepSeek (lightweight).".to_string(),
                                        timestamp: Local::now().format("%H:%M:%S").to_string(),
                                    });
                                }
                                "heavy" => {
                                    self.routing_mode = RoutingMode::ForceHeavy;
                                    self.messages.push(Message {
                                        sender: Sender::Cobolx,
                                        text: "Model routing set to ForceHeavy. All queries sent to GLM-4-Pro (heavy).".to_string(),
                                        timestamp: Local::now().format("%H:%M:%S").to_string(),
                                    });
                                }
                                _ => {
                                    self.messages.push(Message {
                                        sender: Sender::Cobolx,
                                        text: format!("Invalid argument: '{}'. Choose from: auto, light, heavy.", parts[1]),
                                        timestamp: Local::now().format("%H:%M:%S").to_string(),
                                    });
                                }
                            }
                        } else {
                            let mode_str = match self.routing_mode {
                                RoutingMode::Auto => "Auto (Automatic Sub-Agent routing)",
                                RoutingMode::ForceLight => "ForceLight (Forced to DeepSeek)",
                                RoutingMode::ForceHeavy => "ForceHeavy (Forced to GLM-4-Pro)",
                            };
                            self.messages.push(Message {
                                sender: Sender::Cobolx,
                                text: format!("Current routing mode: {}.", mode_str),
                                timestamp: Local::now().format("%H:%M:%S").to_string(),
                            });
                        }
                    }
                    _ => {
                        self.messages.push(Message {
                            sender: Sender::Cobolx,
                            text: format!("Unknown command: /{}. Type /help to see all available commands.", cmd),
                            timestamp: Local::now().format("%H:%M:%S").to_string(),
                        });
                    }
                }
            }
        }

        self.input_text.clear();
        self.show_dropdown = false;
        (should_exit, is_command)
    }
}

pub fn run_tui() -> Result<(), io::Error> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create app state
    let mut app = App::new();

    // Create background task channel
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<TaskUpdate>();

    let res = run_loop(&mut terminal, &mut app, &tx, &mut rx);

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    if let Err(err) = res {
        println!("{:?}", err);
    }

    Ok(())
}

fn trigger_chat_task(
    app: &mut App,
    tx: &tokio::sync::mpsc::UnboundedSender<TaskUpdate>,
) -> bool {
    let raw_text = app.input_text.trim().to_string();
    if raw_text.is_empty() {
        return false;
    }

    let (should_exit, is_command) = app.submit_message();
    if should_exit {
        return true;
    }

    // Spawn sub-agent requests if not a local command
    if !is_command {
        if !app.router.has_credentials() {
            let error_text = if let Some(ref path) = app.router.config_path {
                format!("Error: No API credentials found. Please configure your API keys in the config file at:\n  {}\nOr set DEEPSEEK_API_KEY or GLM_API_KEY environment variables.", path)
            } else {
                "Error: No API credentials found. Please set DEEPSEEK_API_KEY or GLM_API_KEY environment variables.".to_string()
            };
            app.messages.push(Message {
                sender: Sender::Cobolx,
                text: error_text,
                timestamp: Local::now().format("%H:%M:%S").to_string(),
            });
            return false;
        }

        // Add the routing placeholder
        app.messages.push(Message {
            sender: Sender::Cobolx,
            text: "Routing...".to_string(),
            timestamp: Local::now().format("%H:%M:%S").to_string(),
        });

        let router = Arc::clone(&app.router);
        let history = app.messages.clone();
        let mode = app.routing_mode;
        let tx_clone = tx.clone();

        tokio::spawn(async move {
            // Step 1: Sub-Agent Router Classification
            let route = match mode {
                RoutingMode::ForceLight => crate::agent::client::Route::Light,
                RoutingMode::ForceHeavy => crate::agent::client::Route::Heavy,
                RoutingMode::Auto => router.classify_route(&raw_text).await,
            };

            let route_name = match route {
                crate::agent::client::Route::Light => "DeepSeek",
                crate::agent::client::Route::Heavy => "GLM-4-Pro",
            };

            // Update TUI status via channel
            let _ = tx_clone.send(TaskUpdate::Routed(route, route_name));

            // Create streaming channel
            let (stream_tx, mut stream_rx) = tokio::sync::mpsc::unbounded_channel::<String>();

            // Spawn token forwarder
            let tx_clone_delta = tx_clone.clone();
            let route_name_static = match route {
                crate::agent::client::Route::Light => "DeepSeek",
                crate::agent::client::Route::Heavy => "GLM-4-Pro",
            };
            let forward_handle = tokio::spawn(async move {
                while let Some(delta) = stream_rx.recv().await {
                    let _ = tx_clone_delta.send(TaskUpdate::Delta(delta, route_name_static));
                }
            });

            // Step 2: Execute actual dialog query with memory context
            let result = router.execute_chat_stream(&history, route, stream_tx).await;

            // Wait for forwarder to finish
            let _ = forward_handle.await;

            // Update TUI response via channel to mark as finished
            match result {
                Ok((usage, model_used)) => {
                    let _ = tx_clone.send(TaskUpdate::Finished(Ok(usage), model_used));
                }
                Err(err) => {
                    let _ = tx_clone.send(TaskUpdate::Finished(Err(err), "Error"));
                }
            }
        });
    }

    false
}

fn run_loop<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
    tx: &tokio::sync::mpsc::UnboundedSender<TaskUpdate>,
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<TaskUpdate>,
) -> io::Result<()> {
    loop {
        // Drain incoming messages from background task
        while let Ok(update) = rx.try_recv() {
            match update {
                TaskUpdate::Routed(_route, route_name) => {
                    if let Some(msg) = app.messages.iter_mut().last() {
                        if msg.text == "Routing..." {
                            msg.text = format!("(Routed: {}) Thinking...", route_name);
                        }
                    }
                }
                TaskUpdate::Delta(delta, model_used) => {
                    if let Some(msg) = app.messages.iter_mut().last() {
                        if msg.text.contains("Thinking...") {
                            msg.text = format!("(Using {}) {}", model_used, delta);
                        } else {
                            msg.text.push_str(&delta);
                        }
                    }
                }
                TaskUpdate::Finished(res, model_used) => {
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
