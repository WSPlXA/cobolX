use crate::agent::client::AgentRouter;
use crate::ui::draw;
use chrono::Local;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropdownType {
    None,
    Commands,
    Files,
}

pub enum TaskUpdate {
    Routed(crate::agent::client::Route, &'static str),
    Delta(String, &'static str),
    Status(String),
    Finished(
        Result<Option<crate::agent::client::Usage>, String>,
        &'static str,
    ),
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
    pub agent_status: Option<String>,
    pub spinner_tick: usize,
    pub console_scroll_offset: u16,
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
                format!(
                    "Please configure your API keys in the configuration file:\n  {}\nOr input them below directly in this screen.",
                    path
                )
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
            agent_status: None,
            spinner_tick: 0,
            console_scroll_offset: 0,
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
            "/docs".to_string(),
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

    pub fn get_at_query(&self) -> Option<&str> {
        if let Some(idx) = self.input_text.rfind('@') {
            let suffix = &self.input_text[idx + 1..];
            if !suffix.contains(' ') {
                return Some(suffix);
            }
        }
        None
    }

    pub fn get_filtered_files(&self) -> Vec<String> {
        let Some(query) = self.get_at_query() else {
            return Vec::new();
        };
        let query_lower = query.to_lowercase();
        let mut list = Vec::new();
        for entry in &self.discovered_files {
            if let Some(file_name) = entry.path.file_name().and_then(|s| s.to_str()) {
                let file_name_lower = file_name.to_lowercase();
                if query_lower.is_empty() || file_name_lower.contains(&query_lower) {
                    list.push(file_name.to_string());
                }
            }
        }
        list.sort();
        list.dedup();
        list
    }

    pub fn get_dropdown_type(&self) -> DropdownType {
        if self.input_text.starts_with('/') {
            let filtered = self.get_filtered_commands();
            if !filtered.is_empty() {
                return DropdownType::Commands;
            }
        }
        if self.get_at_query().is_some() {
            let filtered = self.get_filtered_files();
            if !filtered.is_empty() {
                return DropdownType::Files;
            }
        }
        DropdownType::None
    }

    pub fn insert_selected_file(&mut self, file_name: &str) {
        if let Some(idx) = self.input_text.rfind('@') {
            let mut new_text = self.input_text[..idx].to_string();
            new_text.push('@');
            new_text.push_str(file_name);
            new_text.push(' ');
            self.input_text = new_text;
        }
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
                                            /docs             - Generate Markdown documentation for all COBOL files into docs/\n\
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
                                text: format!(
                                    "Scanning sandbox directory (recursive): {}",
                                    path.to_string_lossy()
                                ),
                                timestamp: Local::now().format("%H:%M:%S").to_string(),
                            });

                            match crate::memory::MemoryStore::open_or_create(path).and_then(
                                |mut store| {
                                    crate::cobol::indexer::index_sandbox(path, &mut store)
                                        .map(|report| (report, store.db_path().to_path_buf()))
                                },
                            ) {
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
                                }
                                Err(e) => {
                                    self.messages.push(Message {
                                        sender: Sender::Cobolx,
                                        text: format!("Error indexing sandbox: {}", e),
                                        timestamp: Local::now().format("%H:%M:%S").to_string(),
                                    });
                                }
                            }
                        } else {
                            self.messages.push(Message {
                                sender: Sender::Cobolx,
                                text: "No sandbox directory selected.".to_string(),
                                timestamp: Local::now().format("%H:%M:%S").to_string(),
                            });
                        }
                    }
                    "model" => {
                        if parts.len() > 1 {
                            match parts[1].to_lowercase().as_str() {
                                "auto" => {
                                    self.routing_mode = RoutingMode::Auto;
                                    self.messages.push(Message {
                                        sender: Sender::Cobolx,
                                        text: "Routing mode set to Auto.".to_string(),
                                        timestamp: Local::now().format("%H:%M:%S").to_string(),
                                    });
                                }
                                "light" | "lite" => {
                                    self.routing_mode = RoutingMode::ForceLight;
                                    self.messages.push(Message {
                                        sender: Sender::Cobolx,
                                        text: "Routing mode set to Force Lightweight Model (DeepSeek).".to_string(),
                                        timestamp: Local::now().format("%H:%M:%S").to_string(),
                                    });
                                }
                                "heavy" => {
                                    self.routing_mode = RoutingMode::ForceHeavy;
                                    self.messages.push(Message {
                                        sender: Sender::Cobolx,
                                        text: "Routing mode set to Force Heavy Model (GLM-4-Pro)."
                                            .to_string(),
                                        timestamp: Local::now().format("%H:%M:%S").to_string(),
                                    });
                                }
                                _ => {
                                    self.messages.push(Message {
                                        sender: Sender::Cobolx,
                                        text: "Invalid routing mode. Use auto, light, or heavy."
                                            .to_string(),
                                        timestamp: Local::now().format("%H:%M:%S").to_string(),
                                    });
                                }
                            }
                        } else {
                            let current = match self.routing_mode {
                                RoutingMode::Auto => "Auto",
                                RoutingMode::ForceLight => "Force Light (DeepSeek)",
                                RoutingMode::ForceHeavy => "Force Heavy (GLM-4-Pro)",
                            };
                            self.messages.push(Message {
                                sender: Sender::Cobolx,
                                text: format!("Current routing mode: {}", current),
                                timestamp: Local::now().format("%H:%M:%S").to_string(),
                            });
                        }
                    }
                    "config" => {
                        self.view_mode = ViewMode::Config;
                        self.config_active_field = 0;
                    }
                    "tokens" => {
                        let current_routing = match self.routing_mode {
                            RoutingMode::Auto => "Auto",
                            RoutingMode::ForceLight => "Force Light (DeepSeek)",
                            RoutingMode::ForceHeavy => "Force Heavy (GLM-4-Pro)",
                        };
                        let last_model_str = self.last_model.as_deref().unwrap_or("None");
                        let stats = format!(
                            "Token Statistics & Routing Config:\n\
                             ---------------------------------\n\
                             Routing Setting: {}\n\
                             Last Active Model: {}\n\
                             Last Turn Prompt Tokens: {}\n\
                             Last Turn Completion Tokens: {}\n\n\
                             Accumulated DeepSeek Prompt Tokens: {}\n\
                             Accumulated DeepSeek Completion Tokens: {}\n\n\
                             Accumulated GLM-4-Pro Prompt Tokens: {}\n\
                             Accumulated GLM-4-Pro Completion Tokens: {}",
                            current_routing,
                            last_model_str,
                            self.last_prompt_tokens,
                            self.last_completion_tokens,
                            self.deepseek_prompt_tokens,
                            self.deepseek_completion_tokens,
                            self.glm_prompt_tokens,
                            self.glm_completion_tokens
                        );
                        self.messages.push(Message {
                            sender: Sender::Cobolx,
                            text: stats,
                            timestamp: Local::now().format("%H:%M:%S").to_string(),
                        });
                    }
                    "docs" => {
                        if self.sandbox_path.is_none() {
                            self.messages.push(Message {
                                sender: Sender::Cobolx,
                                text: "No sandbox directory set. Please select a sandbox first."
                                    .to_string(),
                                timestamp: Local::now().format("%H:%M:%S").to_string(),
                            });
                        } else if self.discovered_files.is_empty() {
                            self.messages.push(Message {
                                sender: Sender::Cobolx,
                                text: "No COBOL files found. Run /init to scan the sandbox directory first.".to_string(),
                                timestamp: Local::now().format("%H:%M:%S").to_string(),
                            });
                        } else {
                            let sandbox = self.sandbox_path.as_ref().unwrap().clone();

                            // Collect @filename mentions from arguments after /docs
                            let file_mentions: Vec<String> = parts[1..]
                                .iter()
                                .filter(|p| p.starts_with('@'))
                                .map(|p| p.trim_start_matches('@').to_lowercase())
                                .collect();

                            if file_mentions.is_empty() {
                                // ── All-files mode ─────────────────────────────────────
                                let source_count = self.discovered_files.iter()
                                    .filter(|f| f.file_type == crate::cobol::scanner::CobolFileType::Source)
                                    .count();
                                let copy_count = self.discovered_files.iter()
                                    .filter(|f| f.file_type == crate::cobol::scanner::CobolFileType::Copybook)
                                    .count();
                                if let Some(last) = self.messages.last_mut() {
                                    last.text = format!(
                                        "Generate Markdown documentation for all COBOL source files in this project \
                                        ({source_count} source file(s), {copy_count} copybook(s)).\n\n\
                                        STEP 1 — query_sqlite: SELECT id, path FROM files WHERE kind='source' ORDER BY path\n\n\
                                        STEP 2 — For EACH file row (using its id and path):\n\
                                          a) query_sqlite: SELECT id, name FROM programs WHERE file_id=<id>\n\
                                          b) query_sqlite: SELECT name, level, pic, usage_clause, section \
                                             FROM data_items WHERE source_file_id=<id> LIMIT 100\n\
                                          c) query_sqlite: SELECT callee_name, kind FROM call_edges \
                                             WHERE caller_program_id IN (SELECT id FROM programs WHERE file_id=<id>)\n\
                                          d) query_sqlite: SELECT copybook_name, resolve_status \
                                             FROM copybook_uses WHERE from_file_id=<id>\n\
                                          e) read_file: <path>  (read the actual source text)\n\
                                          f) write_file: docs/<basename>.md  (Markdown document)\n\n\
                                        STEP 3 — write_file: docs/README.md  (index listing all programs with links)\n\
                                        Use relative paths for all write_file calls (e.g. docs/MAIN.md)."
                                    );
                                }
                                is_command = false;
                            } else {
                                // ── Specific file(s) mode ──────────────────────────────
                                // Match mentions against discovered_files by filename
                                let matched: Vec<(String, String)> = file_mentions
                                    .iter()
                                    .filter_map(|mention| {
                                        self.discovered_files.iter().find(|f| {
                                            f.path
                                                .file_name()
                                                .and_then(|n| n.to_str())
                                                .map(|n| n.to_lowercase() == *mention)
                                                .unwrap_or(false)
                                        })
                                        .map(|f| {
                                            // Relative path (for DB queries and read_file)
                                            let rel = f.path
                                                .strip_prefix(&sandbox)
                                                .map(|p| p.to_string_lossy().replace('\\', "/"))
                                                .unwrap_or_else(|_| {
                                                    f.path.to_string_lossy().into_owned()
                                                });
                                            let stem = f.path
                                                .file_stem()
                                                .and_then(|s| s.to_str())
                                                .unwrap_or(mention.as_str())
                                                .to_string();
                                            (rel, stem)
                                        })
                                    })
                                    .collect();

                                let not_found: Vec<&str> = file_mentions
                                    .iter()
                                    .filter(|m| {
                                        !matched.iter().any(|(_, stem)| {
                                            stem.to_lowercase() == m.trim_end_matches(".cbl")
                                                .trim_end_matches(".cob")
                                                .trim_end_matches(".cpy")
                                        })
                                    })
                                    .map(|s| s.as_str())
                                    .collect();

                                if matched.is_empty() {
                                    self.messages.push(Message {
                                        sender: Sender::Cobolx,
                                        text: format!(
                                            "File(s) not found in sandbox: {}.\nRun /init first, or check the filename spelling.",
                                            file_mentions.join(", ")
                                        ),
                                        timestamp: Local::now().format("%H:%M:%S").to_string(),
                                    });
                                } else {
                                    let names: Vec<String> =
                                        matched.iter().map(|(_, b)| b.clone()).collect();
                                    let per_file: String = matched
                                        .iter()
                                        .map(|(rel, stem)| {
                                            format!(
                                                "• File: {rel}\n  \
                                                 a) query_sqlite: SELECT id FROM files WHERE path LIKE '%{rel}%'\n  \
                                                 b) query_sqlite: SELECT id, name FROM programs WHERE file_id=<id>\n  \
                                                 c) query_sqlite: SELECT name, level, pic, usage_clause, section \
                                                    FROM data_items WHERE source_file_id=<id>\n  \
                                                 d) query_sqlite: SELECT callee_name, kind FROM call_edges \
                                                    WHERE caller_program_id IN (SELECT id FROM programs WHERE file_id=<id>)\n  \
                                                 e) query_sqlite: SELECT copybook_name, resolve_status \
                                                    FROM copybook_uses WHERE from_file_id=<id>\n  \
                                                 f) read_file: {rel}\n  \
                                                 g) write_file: docs/{stem}.md"
                                            )
                                        })
                                        .collect::<Vec<_>>()
                                        .join("\n\n");

                                    let warn = if not_found.is_empty() {
                                        String::new()
                                    } else {
                                        format!("\n\nNote: file(s) not found in index: {}", not_found.join(", "))
                                    };

                                    if let Some(last) = self.messages.last_mut() {
                                        last.text = format!(
                                            "Generate Markdown documentation for: {names}\n\n\
                                            For EACH file below, follow these steps in order:\n\
                                            {per_file}\n\n\
                                            Each docs/<stem>.md must include:\n\
                                            - Program name and purpose (from IDENTIFICATION DIVISION)\n\
                                            - Data structure summary (levels, PIC clauses, sections — from DB)\n\
                                            - CALL graph and COPY dependencies (from DB)\n\
                                            - Key PROCEDURE paragraphs with brief explanation\n\
                                            - Notable code excerpts from the source\
                                            {warn}",
                                            names = names.join(", ")
                                        );
                                    }
                                    is_command = false;
                                }
                            }
                        }
                    }
                    _ => {
                        self.messages.push(Message {
                            sender: Sender::Cobolx,
                            text: format!("Unknown command: /{}", cmd),
                            timestamp: Local::now().format("%H:%M:%S").to_string(),
                        });
                    }
                }
            }
        }

        self.input_text.clear();
        (should_exit, is_command)
    }
}

fn trigger_chat_task(app: &mut App, tx: &tokio::sync::mpsc::UnboundedSender<TaskUpdate>) -> bool {
    let (should_exit, is_command) = app.submit_message();
    if should_exit {
        return true;
    }
    if is_command {
        return false;
    }

    // Block overlapping agent calls — show error, don't spawn a second task
    if app.active_agent.is_some() {
        app.messages.push(Message {
            sender: Sender::Cobolx,
            text: "Agent is still processing. Please wait until the current response finishes before sending another message.".to_string(),
            timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
        });
        return false;
    }

    // Trigger LLM request
    let router = Arc::clone(&app.router);
    let history = app.messages.clone();
    let routing_mode = app.routing_mode;
    let sandbox_path = app.sandbox_path.clone();
    let tx = tx.clone();

    // Snap console to bottom when user submits a new message
    app.console_scroll_offset = 0;

    // Add a placeholder message for the incoming streaming response
    app.messages.push(Message {
        sender: Sender::Cobolx,
        text: "Thinking...".to_string(),
        timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
    });

    app.active_agent = Some("Router Sub-Agent".to_string());

    tokio::spawn(async move {
        // 1. Classify route
        let route = match routing_mode {
            RoutingMode::ForceLight => crate::agent::client::Route::Light,
            RoutingMode::ForceHeavy => crate::agent::client::Route::Heavy,
            RoutingMode::Auto => {
                let query = history.last().map(|m| m.text.as_str()).unwrap_or("");
                router.classify_route(query).await
            }
        };

        let route_name = match route {
            crate::agent::client::Route::Light => "Lightweight Model (DeepSeek)",
            crate::agent::client::Route::Heavy => "Heavy Model (GLM-4-Pro)",
            crate::agent::client::Route::Database => "Database Sub-Agent",
            crate::agent::client::Route::Filesystem => "Filesystem Sub-Agent",
        };

        let _ = tx.send(TaskUpdate::Routed(route, route_name));

        // 2. Execute chat stream
        let (stream_tx, mut stream_rx) = tokio::sync::mpsc::unbounded_channel::<String>();

        let tx_clone = tx.clone();
        let stream_handle = tokio::spawn(async move {
            while let Some(delta) = stream_rx.recv().await {
                if let Some(status) = delta.strip_prefix("\x01STATUS:") {
                    let _ = tx_clone.send(TaskUpdate::Status(status.to_string()));
                } else {
                    let _ = tx_clone.send(TaskUpdate::Delta(delta, route_name));
                }
            }
        });

        let res = router
            .execute_chat_stream(&history, route, sandbox_path.as_deref(), stream_tx)
            .await;

        let _ = stream_handle.await;

        match res {
            Ok((usage, final_model)) => {
                let _ = tx.send(TaskUpdate::Finished(Ok(usage), final_model));
            }
            Err(e) => {
                let _ = tx.send(TaskUpdate::Finished(Err(e), route_name));
            }
        }
    });

    false
}

pub fn run_tui() -> Result<(), io::Error> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<TaskUpdate>();

    loop {
        // Check for updates
        while let Ok(update) = rx.try_recv() {
            match update {
                TaskUpdate::Routed(ref route, model_used) => {
                    app.active_agent = Some(model_used.to_string());
                    if matches!(
                        route,
                        crate::agent::client::Route::Database
                            | crate::agent::client::Route::Filesystem
                    ) {
                        app.agent_status = Some(format!("Using {}", model_used));
                    }
                    if let Some(msg) = app.messages.iter_mut().last() {
                        if msg.text == "Thinking..." {
                            msg.text = format!("(Routed: {})\nThinking...", model_used);
                        }
                    }
                }
                TaskUpdate::Delta(delta, model_used) => {
                    app.active_agent = Some(model_used.to_string());
                    if let Some(msg) = app.messages.iter_mut().last() {
                        if let Some(reasoning) = delta.strip_prefix("\x01REASONING:") {
                            if msg.text == "Thinking..."
                                || (msg.text.starts_with("(Routed:")
                                    && msg.text.contains("Thinking..."))
                            {
                                msg.text = format!("(Using {}) [Thinking Process]\n{}", model_used, reasoning);
                            } else {
                                msg.text.push_str(reasoning);
                            }
                        } else {
                            if msg.text == "Thinking..."
                                || (msg.text.starts_with("(Routed:")
                                    && msg.text.contains("Thinking..."))
                            {
                                msg.text = format!("(Using {}) {}", model_used, delta);
                            } else {
                                if msg.text.contains("[Thinking Process]") && !msg.text.contains("[Answer]") {
                                    msg.text.push_str("\n[Answer]\n");
                                }
                                msg.text.push_str(&delta);
                            }
                        }
                    }
                }
                TaskUpdate::Status(status) => {
                    if status.is_empty() {
                        app.agent_status = None;
                    } else {
                        app.agent_status = Some(status);
                    }
                }
                TaskUpdate::Finished(res, model_used) => {
                    app.active_agent = None;
                    app.agent_status = None;
                    if let Some(msg) = app.messages.iter_mut().last() {
                        if msg.text == "Thinking..."
                            || (msg.text.starts_with("(Routed:")
                                && msg.text.contains("Thinking..."))
                        {
                            msg.text = format!(
                                "(Using {}) Operation completed successfully.",
                                model_used
                            );
                        }
                    }
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

        if app.agent_status.is_some() {
            app.spinner_tick = app.spinner_tick.wrapping_add(1);
        }
        terminal.draw(|f| draw::draw(f, &mut app))?;

        // Non-blocking poll for crossterm events
        if event::poll(std::time::Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != event::KeyEventKind::Press {
                    continue;
                }
                if key.code == KeyCode::Char('c')
                    && key.modifiers.contains(event::KeyModifiers::CONTROL)
                {
                    break;
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
                        KeyCode::Enter => match app.config_active_field {
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
                                            text: "Configuration successfully saved and reloaded!"
                                                .to_string(),
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
                        },
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
                                std::env::current_dir()
                                    .ok()
                                    .and_then(|p| p.parent().map(|parent| parent.to_path_buf()))
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
                    let dropdown_type = app.get_dropdown_type();
                    let has_options = dropdown_type != DropdownType::None;

                    match key.code {
                        KeyCode::Esc => {
                            if app.show_dropdown {
                                app.show_dropdown = false;
                            } else if app.input_text.is_empty() {
                                break;
                            } else {
                                app.input_text.clear();
                            }
                        }
                        KeyCode::Down | KeyCode::Tab => {
                            if app.show_dropdown && has_options {
                                let len = match dropdown_type {
                                    DropdownType::Commands => app.get_filtered_commands().len(),
                                    DropdownType::Files => app.get_filtered_files().len(),
                                    _ => 0,
                                };
                                if len > 0 {
                                    app.dropdown_index = (app.dropdown_index + 1) % len;
                                }
                            } else if !app.show_dropdown {
                                app.console_scroll_offset =
                                    app.console_scroll_offset.saturating_sub(3);
                            }
                        }
                        KeyCode::Up => {
                            if app.show_dropdown && has_options {
                                let len = match dropdown_type {
                                    DropdownType::Commands => app.get_filtered_commands().len(),
                                    DropdownType::Files => app.get_filtered_files().len(),
                                    _ => 0,
                                };
                                if len > 0 {
                                    app.dropdown_index = (app.dropdown_index + len - 1) % len;
                                }
                            } else if !app.show_dropdown {
                                app.console_scroll_offset =
                                    app.console_scroll_offset.saturating_add(3);
                            }
                        }
                        KeyCode::PageUp => {
                            app.console_scroll_offset =
                                app.console_scroll_offset.saturating_add(10);
                        }
                        KeyCode::PageDown => {
                            app.console_scroll_offset =
                                app.console_scroll_offset.saturating_sub(10);
                        }
                        KeyCode::Enter => {
                            if app.show_dropdown && has_options {
                                match dropdown_type {
                                    DropdownType::Commands => {
                                        let filtered = app.get_filtered_commands();
                                        app.input_text = filtered[app.dropdown_index].clone();
                                        app.show_dropdown = false;
                                        if trigger_chat_task(&mut app, &tx) {
                                            break;
                                        }
                                    }
                                    DropdownType::Files => {
                                        let filtered = app.get_filtered_files();
                                        app.insert_selected_file(&filtered[app.dropdown_index]);
                                        app.show_dropdown = false;
                                    }
                                    _ => {}
                                }
                            } else {
                                if trigger_chat_task(&mut app, &tx) {
                                    break;
                                }
                            }
                        }
                        KeyCode::Char(c) => {
                            app.input_text.push(c);
                            let new_type = app.get_dropdown_type();
                            if new_type != DropdownType::None {
                                app.show_dropdown = true;
                                app.dropdown_index = 0;
                            } else {
                                app.show_dropdown = false;
                            }
                        }
                        KeyCode::Backspace => {
                            app.input_text.pop();
                            let new_type = app.get_dropdown_type();
                            if new_type != DropdownType::None {
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

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    Ok(())
}
