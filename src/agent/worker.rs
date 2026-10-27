//! Runs an agent turn on a background thread so the TUI stays responsive,
//! following the same pattern as `ui::refresh`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::thread;

use super::llm::{ChatMessage, LlmClient};
use super::session::{AgentEvent, run_llm_turn, run_offline_turn};
use super::tools::ToolContext;

pub struct AgentJob {
    pub ctx: ToolContext,
    pub history: Vec<ChatMessage>,
    pub user_text: String,
    /// `None` runs the offline rule engine.
    pub client: Option<Box<dyn LlmClient>>,
}

pub struct AgentHandle {
    pub receiver: Receiver<AgentEvent>,
    cancel: Arc<AtomicBool>,
}

impl AgentHandle {
    /// Ask the worker to stop; it reports [`AgentEvent::Cancelled`].
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

pub fn spawn(job: AgentJob) -> AgentHandle {
    let (sender, receiver) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    thread::spawn(move || {
        let AgentJob {
            mut ctx,
            history,
            user_text,
            client,
        } = job;
        // A closed receiver just means the UI went away; nothing to report.
        let mut emit = |event: AgentEvent| {
            let _ = sender.send(event);
        };
        match client {
            Some(mut client) => run_llm_turn(
                client.as_mut(),
                &mut ctx,
                history,
                &user_text,
                &mut emit,
                &flag,
            ),
            None => run_offline_turn(&ctx, &user_text, super::session::DEFAULT_TOP, &mut emit),
        }
    });
    AgentHandle { receiver, cancel }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::recommend::tests::{conferences, now};
    use crate::agent::{Lang, Profile};
    use crate::data::Favorites;
    use std::time::Duration;

    #[test]
    fn offline_job_reports_until_finished() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ToolContext::new(
            conferences(),
            Favorites::empty_at(dir.path().join("f.json")),
            Profile::default(),
            now(),
            Lang::En,
        );
        let handle = spawn(AgentJob {
            ctx,
            history: Vec::new(),
            user_text: "computer vision".into(),
            client: None,
        });
        let mut saw_recommendations = false;
        loop {
            match handle
                .receiver
                .recv_timeout(Duration::from_secs(10))
                .unwrap()
            {
                AgentEvent::Recommendations(_) => saw_recommendations = true,
                AgentEvent::Finished { .. } => break,
                _ => {}
            }
        }
        assert!(saw_recommendations);
        assert!(!handle.is_cancelled());
    }
}
