mod agent;
mod ui;
mod config;

#[tokio::main]
async fn main() {
    if let Err(e) = ui::tui::run_tui() {
        eprintln!("Error running TUI: {}", e);
        std::process::exit(1);
    }
}
