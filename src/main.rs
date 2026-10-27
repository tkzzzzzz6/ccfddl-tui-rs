use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

use anyhow::{Context, Result};
use chrono::Utc;
use clap::Parser;

use ccfddl_tui::agent::llm::{LlmConfig, OpenAiClient};
use ccfddl_tui::agent::profile::{Profile, default_profile_path, split_list};
use ccfddl_tui::agent::recommend::{Request, recommend};
use ccfddl_tui::agent::report::format_text;
use ccfddl_tui::agent::session::{AgentEvent, run_llm_turn, run_offline_turn};
use ccfddl_tui::agent::tools::ToolContext;
use ccfddl_tui::agent::{Lang, session};
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

    /// Print offline, rule-based conference recommendations and exit
    #[arg(long)]
    recommend: bool,

    /// Ask the recommendation agent one question and exit (uses an LLM when
    /// CCFDDL_API_KEY is set, the offline rules otherwise)
    #[arg(long, value_name = "TEXT")]
    ask: Option<String>,

    /// Research profile to use [default: ~/.ccfddl/profile.yaml]
    #[arg(long, value_name = "PATH")]
    profile: Option<PathBuf>,

    /// Write a commented profile template and exit
    #[arg(long)]
    init_profile: bool,

    /// Extra research interests, comma separated (e.g. "multimodal,LLM")
    #[arg(long, value_name = "LIST")]
    interests: Option<String>,

    /// Number of recommendations
    #[arg(long, value_name = "N", default_value_t = session::DEFAULT_TOP)]
    top: usize,

    /// Chinese output for --recommend / --ask
    #[arg(long)]
    zh: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let profile_path = cli.profile.clone().unwrap_or_else(default_profile_path);
    let lang = if cli.zh { Lang::Zh } else { Lang::En };

    if cli.init_profile {
        return init_profile(&profile_path, lang);
    }

    let mut service = DataService::new(cli.url.clone());

    if cli.list {
        service.load_conferences()?;
        print_list(&service);
        return Ok(());
    }

    if cli.recommend || cli.ask.is_some() {
        service.load_conferences()?;
        let mut profile = Profile::load(&profile_path)?.unwrap_or_default();
        if let Some(extra) = &cli.interests {
            profile = profile.with_extra_interests(&split_list(extra));
        }
        return match &cli.ask {
            Some(question) => ask(&service, profile, question, cli.top, lang),
            None => print_recommendations(&service, &profile, &profile_path, cli.top, lang),
        };
    }

    // The interactive UI performs the initial load in the background so the
    // interface appears immediately and remains responsive during refreshes.
    ui::run(service, profile_path)?;
    Ok(())
}

fn init_profile(path: &std::path::Path, lang: Lang) -> Result<()> {
    if path.exists() {
        println!(
            "{} {}",
            lang.pick("Profile already exists:", "画像文件已存在："),
            path.display()
        );
        return Ok(());
    }
    Profile::default()
        .save(path)
        .with_context(|| format!("writing {}", path.display()))?;
    println!(
        "{} {}",
        lang.pick(
            "Wrote profile template, edit it and run --recommend:",
            "已生成画像模板，编辑后运行 --recommend："
        ),
        path.display()
    );
    Ok(())
}

fn print_recommendations(
    service: &DataService,
    profile: &Profile,
    profile_path: &std::path::Path,
    top: usize,
    lang: Lang,
) -> Result<()> {
    let now = Utc::now();
    if profile.is_empty() {
        eprintln!(
            "{} {}",
            lang.pick(
                "Hint: no research interests set. Use --interests or create a profile with --init-profile:",
                "提示：尚未设置研究方向。可使用 --interests，或用 --init-profile 创建画像："
            ),
            profile_path.display()
        );
    }
    let set = recommend(
        &Request {
            conferences: service.conferences(),
            favorites: service.favorites(),
            profile,
            now,
            lang,
        },
        top,
    );
    print!("{}", format_text(&set, now, lang));
    Ok(())
}

fn ask(
    service: &DataService,
    profile: Profile,
    question: &str,
    top: usize,
    lang: Lang,
) -> Result<()> {
    let now = Utc::now();
    let mut ctx = ToolContext::new(
        service.conferences().to_vec(),
        service.favorites().clone(),
        profile,
        now,
        lang,
    );
    let mut result = None;
    let mut failure = None;
    let mut emit = |event: AgentEvent| match event {
        AgentEvent::Delta(text) => {
            print!("{text}");
            let _ = std::io::stdout().flush();
        }
        AgentEvent::ToolStarted { name, .. } => eprintln!("\n⏺ {name}"),
        AgentEvent::ToolFinished { summary, .. } => eprintln!("  ⎿ {summary}"),
        AgentEvent::Recommendations(set) => result = Some(set),
        AgentEvent::Failed(error) => failure = Some(error),
        AgentEvent::Cancelled | AgentEvent::Finished { .. } => {}
    };
    match LlmConfig::from_env() {
        Some(config) => {
            let mut client = OpenAiClient::new(config);
            run_llm_turn(
                &mut client,
                &mut ctx,
                Vec::new(),
                question,
                &mut emit,
                &AtomicBool::new(false),
            );
        }
        None => run_offline_turn(&ctx, question, top, &mut emit),
    }
    println!();
    if let Some(set) = result {
        println!();
        print!("{}", format_text(&set, now, lang));
    }
    if let Some(error) = failure {
        anyhow::bail!("agent failed: {error}");
    }
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
