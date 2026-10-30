use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

use anyhow::{Context, Result};
use chrono::Utc;
use clap::Parser;

use ccfddl_tui::agent::cli_setup::{self, Io};
use ccfddl_tui::agent::config::{Overrides, default_config_path, load_and_resolve};
use ccfddl_tui::agent::llm::{LlmClient, LlmConfig, OpenAiClient, list_models};
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

    /// Chinese output for --recommend / --ask / --setup / --check-llm
    #[arg(long)]
    zh: bool,

    /// Colours: auto (default), truecolor, 256, 16 or none. Also CCFDDL_COLOR;
    /// NO_COLOR is honoured
    #[arg(
        long,
        value_name = "MODE",
        value_parser = ["auto", "truecolor", "256", "16", "none"]
    )]
    color: Option<String>,

    /// Connect a model interactively (provider, API key, model) and exit
    #[arg(long)]
    setup: bool,

    /// Show the effective model settings, list models, send a test request
    #[arg(long)]
    check_llm: bool,

    /// Model settings file [default: ~/.ccfddl/config.yaml]
    #[arg(long, value_name = "PATH")]
    config: Option<PathBuf>,

    /// Use this saved model configuration from config.yaml
    #[arg(long, value_name = "NAME")]
    llm_profile: Option<String>,

    /// Provider preset for this run (deepseek, qwen, kimi, zhipu, siliconflow,
    /// openrouter, openai, ollama, custom)
    #[arg(long, value_name = "ID")]
    provider: Option<String>,

    /// Model name for this run
    #[arg(long, value_name = "NAME")]
    model: Option<String>,

    /// OpenAI-compatible base URL for this run
    #[arg(long, value_name = "URL")]
    base_url: Option<String>,
}

fn env_lookup(key: &str) -> Option<String> {
    std::env::var(key).ok()
}

/// Real network and terminal implementations for the setup prompts.
fn with_terminal_io<T>(f: impl FnOnce(&mut Io) -> T) -> T {
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    let mut out = std::io::stdout();
    let mut read_secret = read_hidden_line;
    let hidden = std::io::IsTerminal::is_terminal(&std::io::stdin());
    let fetch = |config: &LlmConfig| list_models(config);
    let make =
        |config: &LlmConfig| -> Box<dyn LlmClient> { Box::new(OpenAiClient::new(config.clone())) };
    let mut io = Io {
        input: &mut input,
        out: &mut out,
        read_secret: if hidden { Some(&mut read_secret) } else { None },
        env: &env_lookup,
        fetch_models: &fetch,
        make_client: &make,
    };
    f(&mut io)
}

/// Read a line without echoing it (bullets are shown instead). Only used
/// when stdin is a terminal; piped input is read as plain lines.
fn read_hidden_line(out: &mut dyn Write) -> std::io::Result<String> {
    use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
    use ratatui::crossterm::terminal;

    // stdin is locked by the caller, so there is no plain-line fallback here.
    terminal::enable_raw_mode().map_err(|e| {
        std::io::Error::other(format!("cannot hide input ({e}); use env:VAR_NAME instead"))
    })?;
    let mut secret = String::new();
    let result = loop {
        match event::read() {
            Ok(Event::Key(key)) if key.kind != KeyEventKind::Release => match key.code {
                KeyCode::Enter => break Ok(()),
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    break Err(std::io::Error::new(
                        std::io::ErrorKind::Interrupted,
                        "cancelled",
                    ));
                }
                KeyCode::Backspace => {
                    if secret.pop().is_some() {
                        let _ = write!(out, "\u{8} \u{8}");
                    }
                }
                KeyCode::Char(c) => {
                    secret.push(c);
                    let _ = write!(out, "•");
                }
                _ => {}
            },
            // Pasted text arrives as one event in some terminals.
            Ok(Event::Paste(text)) => {
                let _ = write!(out, "{}", "•".repeat(text.chars().count()));
                secret.push_str(&text);
            }
            Ok(_) => {}
            Err(error) => break Err(error),
        }
        let _ = out.flush();
    };
    let _ = terminal::disable_raw_mode();
    let _ = writeln!(out);
    result.map(|()| secret)
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let profile_path = cli.profile.clone().unwrap_or_else(default_profile_path);
    let lang = if cli.zh { Lang::Zh } else { Lang::En };

    let config_path = cli.config.clone().unwrap_or_else(default_config_path);
    let overrides = Overrides {
        profile: cli.llm_profile.clone(),
        provider: cli.provider.clone(),
        model: cli.model.clone(),
        base_url: cli.base_url.clone(),
    };

    if cli.init_profile {
        return init_profile(&profile_path, lang);
    }
    if cli.setup {
        with_terminal_io(|io| cli_setup::run_setup(io, &config_path, lang))?;
        return Ok(());
    }
    if cli.check_llm {
        let ok = with_terminal_io(|io| cli_setup::check(io, &config_path, &overrides, lang))?;
        if !ok {
            std::process::exit(1);
        }
        return Ok(());
    }

    // A broken model configuration never blocks the app: report it and run
    // the offline rules instead.
    let (llm, llm_error) = match load_and_resolve(&config_path, &overrides, &env_lookup) {
        Ok(resolved) => (resolved.map(|r| r.config), None),
        Err(error) => (None, Some(error.to_string())),
    };

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
            Some(question) => {
                if let Some(error) = &llm_error {
                    eprintln!("{error}");
                }
                ask(&service, profile, llm, question, cli.top, lang)
            }
            None => print_recommendations(&service, &profile, &profile_path, cli.top, lang),
        };
    }

    // The interactive UI performs the initial load in the background so the
    // interface appears immediately and remains responsive during refreshes.
    let colors = ui::color_mode::resolve(cli.color.as_deref(), &|key| std::env::var(key).ok())
        .map_err(|error| anyhow::anyhow!(error))?;
    let agent = ui::agent::AgentPanel::new(profile_path, llm)
        .with_config_path(config_path)
        .with_overrides(overrides)
        .with_notice(llm_error);
    ui::run(service, agent, colors)?;
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
    llm: Option<LlmConfig>,
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
    match llm {
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
