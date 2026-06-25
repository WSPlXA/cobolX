mod agent;
mod ui;
mod config;
mod cobol;
mod memory;


#[tokio::main]
async fn main() {
    if let Ok(root) = std::env::current_dir() {
        if let Err(e) = memory::MemoryStore::open_or_create(root) {
            eprintln!("Error initializing memory store: {}", e);
            std::process::exit(1);
        }
    }

    if let Err(e) = ui::tui::run_tui() {
        eprintln!("Error running TUI: {}", e);
        std::process::exit(1);
    }
}
