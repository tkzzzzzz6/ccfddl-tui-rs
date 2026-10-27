//! Plain-text and Markdown rendering of a [`RecommendationSet`].

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Local, Utc};

use super::Lang;
use super::recommend::{Recommendation, RecommendationSet, Source, Tier};

fn days_label(item: &Recommendation, now: DateTime<Utc>, lang: Lang) -> String {
    match item.facts.days_left(now) {
        Some(days) => match lang {
            Lang::Zh => format!("剩 {days} 天"),
            Lang::En => format!("{days}d left"),
        },
        None => lang.pick("TBD", "待定").to_string(),
    }
}

fn source_label(source: &Source, lang: Lang) -> String {
    match source {
        Source::Rules => lang.pick("offline rules", "离线规则").to_string(),
        Source::Llm(model) => format!("LLM ({model})"),
    }
}

/// Terminal output for `--recommend` / `--ask`.
pub fn format_text(set: &RecommendationSet, now: DateTime<Utc>, lang: Lang) -> String {
    let mut out = String::new();
    out.push_str(&set.summary);
    out.push('\n');
    let mut current: Option<Tier> = None;
    for (index, item) in set.items.iter().enumerate() {
        if current != Some(item.tier) {
            current = Some(item.tier);
            out.push_str(&format!("\n== {} ==\n", item.tier.label(lang)));
        }
        let facts = &item.facts;
        out.push_str(&format!(
            "{}. {} {}  [{}]  {}  {}\n",
            index + 1,
            facts.title,
            facts.year,
            facts.rank_label(),
            facts.sub,
            days_label(item, now, lang)
        ));
        out.push_str(&format!(
            "   {}: {}\n",
            lang.pick("Deadline", "截稿"),
            facts.deadline_text
        ));
        if !facts.place.is_empty() || !facts.date.is_empty() {
            out.push_str(&format!(
                "   {}: {}  {}\n",
                lang.pick("Venue", "会议"),
                facts.date,
                facts.place
            ));
        }
        out.push_str(&format!(
            "   {}: {}\n",
            lang.pick("Why", "理由"),
            item.reason
        ));
        out.push_str(&format!(
            "   {}: {}\n",
            lang.pick("Direction", "方向"),
            item.direction
        ));
        if !item.risks.is_empty() {
            out.push_str(&format!(
                "   {}: {}\n",
                lang.pick("Risks", "风险"),
                item.risks.join(lang.pick("; ", "；"))
            ));
        }
        if !facts.link.is_empty() {
            out.push_str(&format!("   {}\n", facts.link));
        }
    }
    out.push_str(&format!(
        "\n({}: {})\n",
        lang.pick("Source", "来源"),
        source_label(&set.source, lang)
    ));
    out
}

fn md_escape(text: &str) -> String {
    text.replace('|', "\\|").replace('\n', " ")
}

/// Markdown report for `/export`.
pub fn format_markdown(set: &RecommendationSet, now: DateTime<Utc>, lang: Lang) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "# {}\n\n",
        lang.pick("Conference recommendations", "会议推荐")
    ));
    out.push_str(&format!(
        "- {}: {}\n- {}: {}\n\n{}\n\n",
        lang.pick("Generated", "生成时间"),
        set.generated_at
            .with_timezone(&Local)
            .format("%Y-%m-%d %H:%M"),
        lang.pick("Source", "来源"),
        source_label(&set.source, lang),
        set.summary
    ));
    out.push_str(&format!(
        "| {} | {} | {} | {} | {} |\n|---|---|---|---|---|\n",
        lang.pick("Tier", "档位"),
        lang.pick("Conference", "会议"),
        lang.pick("Rank", "等级"),
        lang.pick("Deadline", "截稿"),
        lang.pick("Left", "剩余"),
    ));
    for item in &set.items {
        out.push_str(&format!(
            "| {} | {} {} | {} | {} | {} |\n",
            item.tier.label(lang),
            md_escape(&item.facts.title),
            item.facts.year,
            item.facts.rank_label(),
            md_escape(&item.facts.deadline_text),
            days_label(item, now, lang)
        ));
    }
    for item in &set.items {
        let facts = &item.facts;
        out.push_str(&format!(
            "\n## {} {} · {}\n\n",
            facts.title,
            facts.year,
            item.tier.label(lang)
        ));
        if !facts.description.is_empty() {
            out.push_str(&format!("*{}*\n\n", facts.description));
        }
        out.push_str(&format!(
            "- **{}**: {}\n",
            lang.pick("Why", "推荐理由"),
            item.reason
        ));
        out.push_str(&format!(
            "- **{}**: {}\n",
            lang.pick("Direction", "投稿方向"),
            item.direction
        ));
        if !item.risks.is_empty() {
            out.push_str(&format!(
                "- **{}**: {}\n",
                lang.pick("Risks", "风险"),
                item.risks.join(lang.pick("; ", "；"))
            ));
        }
        if !facts.date.is_empty() || !facts.place.is_empty() {
            out.push_str(&format!(
                "- **{}**: {} {}\n",
                lang.pick("Venue", "会议"),
                facts.date,
                facts.place
            ));
        }
        if !facts.link.is_empty() {
            out.push_str(&format!("- {}\n", facts.link));
        }
    }
    out
}

/// Write the Markdown report. `target` may be a file or a directory; a
/// directory gets a timestamped file name.
pub fn export_markdown(
    set: &RecommendationSet,
    target: &Path,
    now: DateTime<Utc>,
    lang: Lang,
) -> io::Result<PathBuf> {
    let path = if target.extension().is_some_and(|ext| ext == "md") {
        target.to_path_buf()
    } else {
        target.join(format!(
            "recommendations-{}.md",
            now.with_timezone(&Local).format("%Y%m%d-%H%M%S")
        ))
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, format_markdown(set, now, lang))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::Profile;
    use crate::agent::recommend::tests::{conferences, now};
    use crate::agent::recommend::{Request, recommend};
    use crate::data::Favorites;

    fn sample(lang: Lang) -> RecommendationSet {
        let dir = tempfile::tempdir().unwrap();
        let favorites = Favorites::empty_at(dir.path().join("f.json"));
        let confs = conferences();
        let profile = Profile {
            interests: vec!["computer vision".into()],
            ..Profile::default()
        };
        recommend(
            &Request {
                conferences: &confs,
                favorites: &favorites,
                profile: &profile,
                now: now(),
                lang,
            },
            5,
        )
    }

    #[test]
    fn text_report_groups_by_tier() {
        let text = format_text(&sample(Lang::En), now(), Lang::En);
        assert!(text.contains("== Stretch =="), "{text}");
        assert!(text.contains("CVPR 2030"), "{text}");
        assert!(
            text.contains("Deadline: 2030-03-10 23:59:59 (UTC-12)"),
            "{text}"
        );
        assert!(text.contains("offline rules"), "{text}");
    }

    #[test]
    fn markdown_export_writes_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let set = sample(Lang::Zh);
        let path = export_markdown(&set, dir.path(), now(), Lang::Zh).unwrap();
        let content = fs::read_to_string(path).unwrap();
        assert!(content.starts_with("# 会议推荐"));
        assert!(content.contains("| 冲刺 | CVPR 2030 |"), "{content}");
        assert!(content.contains("**投稿方向**"));

        let explicit = dir.path().join("out/report.md");
        assert_eq!(
            export_markdown(&set, &explicit, now(), Lang::Zh).unwrap(),
            explicit
        );
    }
}
