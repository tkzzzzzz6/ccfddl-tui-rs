use anyhow::Result;
use chrono::Utc;
use clap::Parser;

use ccfddl_tui::data::{DataService, query};
use ccfddl_tui::ui;

#[derive(Parser, Debug)]
#[command(
    name = "ccfddl-tui",
    version,
    about = "Track CCF-ranked conference deadlines"
)]
struct Cli {
    /// Custom conference data URL
    #[arg(long, value_name = "URL")]
    url: Option<String>,

    /// Print the conference list and exit instead of starting the TUI
    #[arg(long)]
    list: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let mut service = DataService::new(cli.url);

    if cli.list {
        service.load_conferences()?;
        print_list(&service);
        return Ok(());
    }

    // The interactive UI performs the initial load in the background so the
    // interface appears immediately and remains responsive during refreshes.
    ui::run(service)?;
    Ok(())
}

fn print_list(service: &DataService) {
    let rows = query::visible_rows(
        &service.process_rows(Utc::now()),
        &query::FilterState::default(),
    );
    let local = if service.using_local_data() {
        " [LOCAL]"
    } else {
        ""
    };
    println!("Showing {} conferences{local}", rows.len());
    for row in &rows {
        println!(
            "{:<24} {:>4}  CCF {:<2}  {:<18}  {}",
            row.title, row.year, row.rank, row.countdown, row.place
        );
    }
}
