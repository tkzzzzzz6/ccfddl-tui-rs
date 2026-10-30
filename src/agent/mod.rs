//! Conference recommendation agent.
//!
//! The agent core does not depend on the terminal UI, so the same code serves
//! `--recommend`, `--ask` and the TUI panel:
//!
//! - [`recommend`]: offline rule-based scoring (P0, needs no LLM);
//! - [`config`] / [`providers`]: LLM settings from `config.yaml`, environment
//!   and CLI flags, with built-in provider presets;
//! - [`llm`]: OpenAI-compatible chat client with tool calling and streaming;
//! - [`tools`]: tools the model calls, all backed by the local data layer;
//! - [`session`]: the agent loop (or the offline fallback) for one user turn;
//! - [`worker`]: runs a turn on a background thread for the TUI.
//!
//! Facts shown to the user (deadlines, ranks, places) always come from the
//! data layer; the model only contributes reasons, directions and risks.

pub mod cli_setup;
pub mod config;
pub mod explain;
pub mod history_store;
pub mod llm;
pub mod profile;
pub mod providers;
pub mod recommend;
pub mod refine;
pub mod report;
pub mod session;
pub mod session_store;
pub mod tools;
pub mod topics;
pub mod worker;

pub use profile::Profile;
pub use recommend::{Recommendation, RecommendationSet, Tier};

/// Output language of the agent, mirroring the UI's EN/ZH toggle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Lang {
    #[default]
    En,
    Zh,
}

impl Lang {
    pub fn is_zh(self) -> bool {
        self == Self::Zh
    }

    /// Pick the English or Chinese variant of a message.
    pub fn pick<'a>(self, en: &'a str, zh: &'a str) -> &'a str {
        match self {
            Self::En => en,
            Self::Zh => zh,
        }
    }
}
