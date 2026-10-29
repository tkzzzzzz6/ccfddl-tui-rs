//! Agent input conveniences: the slash-command table, command completion and
//! input history. Pure state on [`AgentPanel`]; `input.rs` routes keys here
//! and `agent_view.rs` draws the completion popup.

use crate::agent::Lang;

use super::agent::AgentPanel;

/// One slash command, shared by `/help` and the completion popup.
pub struct SlashCommand {
    /// Without the leading `/`.
    pub name: &'static str,
    pub args: &'static str,
    pub en: &'static str,
    pub zh: &'static str,
}

impl SlashCommand {
    pub fn usage(&self) -> String {
        if self.args.is_empty() {
            format!("/{}", self.name)
        } else {
            format!("/{} {}", self.name, self.args)
        }
    }

    pub fn description(&self, lang: Lang) -> &'static str {
        lang.pick(self.en, self.zh)
    }
}

pub const SLASH_COMMANDS: &[SlashCommand] = &[
    SlashCommand {
        name: "recommend",
        args: "[note]",
        en: "Recommend for your profile",
        zh: "按画像推荐（可附加说明）",
    },
    SlashCommand {
        name: "init",
        args: "",
        en: "Set up the research profile (form)",
        zh: "设置研究画像（表单）",
    },
    SlashCommand {
        name: "profile",
        args: "",
        en: "View and edit the profile",
        zh: "查看并编辑画像",
    },
    SlashCommand {
        name: "export",
        args: "[path]",
        en: "Save recommendations as Markdown",
        zh: "导出推荐为 Markdown",
    },
    SlashCommand {
        name: "setup",
        args: "",
        en: "Connect a model (provider, key, model)",
        zh: "配置模型（厂商、Key、模型）",
    },
    SlashCommand {
        name: "model",
        args: "[name | list | set]",
        en: "Show, switch or list models",
        zh: "查看、切换或列出模型",
    },
    SlashCommand {
        name: "config",
        args: "",
        en: "Effective settings and their sources",
        zh: "查看生效配置及来源",
    },
    SlashCommand {
        name: "lang",
        args: "",
        en: "Switch English / Chinese",
        zh: "切换中英文",
    },
    SlashCommand {
        name: "only",
        args: "",
        en: "Table shows only the recommendations (again: all)",
        zh: "表格只显示推荐的会议（再次执行恢复）",
    },
    SlashCommand {
        name: "reset",
        args: "",
        en: "Forget follow-up conditions (rank, count, exclusions…)",
        zh: "清空追问条件（等级、数量、排除…）",
    },
    SlashCommand {
        name: "forget",
        args: "",
        en: "Delete the saved session (history, last cards)",
        zh: "删除保存的会话（输入历史、上次推荐）",
    },
    SlashCommand {
        name: "clear",
        args: "",
        en: "Clear the conversation",
        zh: "清空对话",
    },
    SlashCommand {
        name: "help",
        args: "",
        en: "Commands and keys",
        zh: "命令与按键说明",
    },
];

/// Sent inputs kept for Up/Down.
pub const HISTORY_LIMIT: usize = 100;

impl AgentPanel {
    /// Remember a sent input, skipping consecutive duplicates.
    pub fn record_history(&mut self, text: &str) {
        let text = text.trim();
        if text.is_empty() || self.input_history.last().is_some_and(|last| last == text) {
            return;
        }
        self.input_history.push(text.to_string());
        if self.input_history.len() > HISTORY_LIMIT {
            self.input_history.remove(0);
        }
    }

    /// Up: show the previous sent input, keeping the current draft.
    pub fn history_prev(&mut self) {
        if self.input_history.is_empty() {
            return;
        }
        let index = match self.history_index {
            None => {
                self.history_draft = self.input.text();
                self.input_history.len() - 1
            }
            Some(0) => 0,
            Some(i) => i - 1,
        };
        self.history_index = Some(index);
        self.input.set(&self.input_history[index]);
    }

    /// Down: move towards newer inputs, ending with the draft.
    pub fn history_next(&mut self) {
        let Some(index) = self.history_index else {
            return;
        };
        if index + 1 < self.input_history.len() {
            self.history_index = Some(index + 1);
            self.input.set(&self.input_history[index + 1]);
        } else {
            self.history_index = None;
            let draft = std::mem::take(&mut self.history_draft);
            self.input.set(&draft);
        }
    }

    /// Call after any edit: editing leaves history browsing and re-opens a
    /// dismissed completion popup.
    pub fn input_edited(&mut self) {
        self.history_index = None;
        self.completion_dismissed = false;
        self.completion_cursor = 0;
    }

    /// Commands matching the typed `/prefix`, while the popup applies:
    /// typing the command name, outside `/setup` and `/init`.
    pub fn completions(&self) -> Vec<&'static SlashCommand> {
        // No popup over a recalled history entry: Up/Down keep browsing.
        if self.completion_dismissed
            || self.history_index.is_some()
            || self.setup.is_some()
            || self.form.is_some()
        {
            return Vec::new();
        }
        let text = self.input.text();
        let Some(prefix) = text.strip_prefix('/') else {
            return Vec::new();
        };
        if prefix.contains(char::is_whitespace) {
            return Vec::new();
        }
        let prefix = prefix.to_lowercase();
        SLASH_COMMANDS
            .iter()
            .filter(|c| c.name.starts_with(&prefix))
            .collect()
    }

    pub fn completion_visible(&self) -> bool {
        !self.completions().is_empty()
    }

    pub fn move_completion(&mut self, delta: isize) {
        let count = self.completions().len();
        if count == 0 {
            return;
        }
        let next = self.completion_cursor as isize + delta;
        self.completion_cursor = next.rem_euclid(count as isize) as usize;
    }

    /// Tab: complete the selected command. Commands with arguments get a
    /// trailing space so the argument can be typed straight away.
    pub fn accept_completion(&mut self) {
        let matches = self.completions();
        let Some(command) =
            matches.get(self.completion_cursor.min(matches.len().saturating_sub(1)))
        else {
            return;
        };
        let mut text = format!("/{}", command.name);
        if !command.args.is_empty() {
            text.push(' ');
        }
        self.input.set(&text);
        self.history_index = None;
        self.completion_cursor = 0;
        // A complete command without arguments needs no popup any more.
        self.completion_dismissed = command.args.is_empty();
    }

    pub fn dismiss_completion(&mut self) {
        self.completion_dismissed = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn panel() -> AgentPanel {
        AgentPanel::new(PathBuf::from("/nonexistent/profile.yaml"), None)
    }

    #[test]
    fn command_names_are_unique() {
        let mut names: Vec<&str> = SLASH_COMMANDS.iter().map(|c| c.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), SLASH_COMMANDS.len());
    }

    #[test]
    fn history_walks_back_and_restores_the_draft() {
        let mut p = panel();
        for text in ["first", "second", "second", "  "] {
            p.record_history(text);
        }
        assert_eq!(p.input_history, ["first", "second"]);
        p.input.set("draft");
        p.history_prev();
        assert_eq!(p.input, "second");
        p.history_prev();
        p.history_prev();
        assert_eq!(p.input, "first");
        p.history_next();
        assert_eq!(p.input, "second");
        p.history_next();
        assert_eq!(p.input, "draft");
        // Down without browsing does nothing.
        p.history_next();
        assert_eq!(p.input, "draft");
    }

    #[test]
    fn history_is_bounded() {
        let mut p = panel();
        for i in 0..HISTORY_LIMIT + 5 {
            p.record_history(&format!("q{i}"));
        }
        assert_eq!(p.input_history.len(), HISTORY_LIMIT);
        assert_eq!(p.input_history[0], "q5");
    }

    #[test]
    fn completion_filters_moves_and_completes() {
        let mut p = panel();
        p.input.set("/");
        assert_eq!(p.completions().len(), SLASH_COMMANDS.len());
        p.input.set("/ex");
        p.input_edited();
        let names: Vec<&str> = p.completions().iter().map(|c| c.name).collect();
        assert_eq!(names, ["export"]);
        p.accept_completion();
        assert_eq!(p.input, "/export ");
        // Typing the argument closes the popup.
        assert!(!p.completion_visible());

        p.input.set("/c");
        p.input_edited();
        let names: Vec<&str> = p.completions().iter().map(|c| c.name).collect();
        assert_eq!(names, ["config", "clear"]);
        p.move_completion(1);
        p.accept_completion();
        assert_eq!(p.input, "/clear");
        assert!(!p.completion_visible());
        // Wrapping selection.
        p.input.set("/c");
        p.input_edited();
        p.move_completion(-1);
        assert_eq!(p.completion_cursor, 1);
    }

    #[test]
    fn completion_can_be_dismissed_and_reopens_on_edit() {
        let mut p = panel();
        p.input.set("/he");
        assert!(p.completion_visible());
        p.dismiss_completion();
        assert!(!p.completion_visible());
        p.input.insert_char('l');
        p.input_edited();
        assert!(p.completion_visible());
        p.input.set("hello /he");
        assert!(!p.completion_visible());
    }

    #[test]
    fn recalled_commands_do_not_open_the_popup() {
        let mut p = panel();
        p.record_history("/help");
        p.record_history("/profile");
        p.history_prev();
        assert_eq!(p.input, "/profile");
        assert!(!p.completion_visible());
        p.history_prev();
        assert_eq!(p.input, "/help");
        // Editing the recalled text brings the popup back.
        p.input.backspace();
        p.input_edited();
        assert!(p.completion_visible());
    }
}
