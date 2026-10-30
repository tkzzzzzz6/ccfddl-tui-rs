//! The research-profile form opened by `/init` and `/profile`.
//!
//! Every field is visible at once and can be revisited: text fields for the
//! interests and the ready date, check boxes and radio buttons for the rest.
//! A live preview runs the offline scorer on the draft. Pure logic; drawn by
//! `profile_form_view.rs`.

use chrono::{Datelike, NaiveDate};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::agent::Lang;
use crate::agent::profile::{Profile, parse_flexible_date, split_list};
use crate::agent::refine::{Region, regions};
use crate::agent::topics::{tag_name, tags_for_text};
use crate::model::CATEGORIES;

use super::line_input::LineInput;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Interests,
    TargetRanks,
    Categories,
    ReadyDate,
    Horizon,
    IncludeTbd,
    PreferredPlaces,
    AvoidedPlaces,
}

impl Field {
    pub const ALL: [Field; 8] = [
        Self::Interests,
        Self::TargetRanks,
        Self::Categories,
        Self::ReadyDate,
        Self::Horizon,
        Self::IncludeTbd,
        Self::PreferredPlaces,
        Self::AvoidedPlaces,
    ];

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|f| *f == self).unwrap_or(0)
    }

    pub fn label(self, lang: Lang) -> &'static str {
        match self {
            Self::Interests => lang.pick("Interests", "研究方向"),
            Self::TargetRanks => lang.pick("Ranks", "目标等级"),
            Self::Categories => lang.pick("Category", "CCF 分类"),
            Self::ReadyDate => lang.pick("Ready", "论文完成"),
            Self::Horizon => lang.pick("Horizon", "截稿范围"),
            Self::IncludeTbd => lang.pick("TBD", "待定会议"),
            Self::PreferredPlaces => lang.pick("Prefer", "偏好地点"),
            Self::AvoidedPlaces => lang.pick("Avoid", "避开地点"),
        }
    }

    pub fn is_text(self) -> bool {
        matches!(self, Self::Interests | Self::ReadyDate)
    }
}

pub const RANKS: [&str; 4] = ["A", "B", "C", "N"];
/// Horizon presets in days.
pub const HORIZONS: [(i64, &str, &str); 4] = [
    (90, "3 months", "3 个月"),
    (182, "6 months", "6 个月"),
    (365, "1 year", "1 年"),
    (730, "2 years", "2 年"),
];

/// One choice in a check box or radio row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    pub label: String,
    pub checked: bool,
    /// Covered by a larger checked region (Japan when Asia is checked).
    pub implied: bool,
}

#[derive(Debug, Clone)]
pub struct ProfileForm {
    pub field: Field,
    /// Cursor within a row of choices.
    pub option: usize,
    pub interests: LineInput,
    pub ready: LineInput,
    pub draft: Profile,
    /// The profile the form started from, to tell what changed.
    pub original: Profile,
    /// Shown above the fields, e.g. what `/profile save` changed (#33).
    pub notice: Option<String>,
    pub error: Option<String>,
    /// Run a recommendation after saving (first use from shortcut 1).
    pub recommend_after_save: bool,
}

/// The ready date field holds text that is not a date.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidDate;

/// What a key did, for the caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormOutcome {
    Continue,
    Save,
    Cancel,
}

fn region_selected(region: &Region, places: &[String]) -> bool {
    region.3.iter().all(|k| places.iter().any(|p| p == k))
}

/// Region names for a place list (largest regions first), then any places
/// no region covers: `China, Hong Kong, Macau, Macao` reads `国内`.
pub fn place_names(places: &[String], lang: Lang) -> Vec<String> {
    let mut covered: Vec<&str> = Vec::new();
    let mut names = Vec::new();
    let mut by_size: Vec<&Region> = regions().iter().collect();
    by_size.sort_by_key(|r| std::cmp::Reverse(r.3.len()));
    for region in by_size {
        if region_selected(region, places) && !region.3.iter().all(|k| covered.contains(k)) {
            covered.extend(region.3.iter().copied());
            names.push(lang.pick(region.1, region.0).to_string());
        }
    }
    names.extend(
        places
            .iter()
            .filter(|p| !covered.contains(&p.as_str()))
            .cloned(),
    );
    names
}

impl ProfileForm {
    /// A form for `draft` that reports changes against `original`.
    pub fn with_draft(original: &Profile, draft: &Profile) -> Self {
        let mut form = Self::new(draft);
        form.original = original.clone();
        form
    }

    pub fn new(profile: &Profile) -> Self {
        let mut interests = LineInput::new();
        interests.set(&profile.interests.join(", "));
        let mut ready = LineInput::new();
        if let Some(date) = profile.ready_date {
            ready.set(&date.to_string());
        }
        Self {
            field: Field::Interests,
            option: 0,
            interests,
            ready,
            draft: profile.clone(),
            original: profile.clone(),
            notice: None,
            error: None,
            recommend_after_save: false,
        }
    }

    /// Choices of a check box / radio field, with their state.
    pub fn choices(&self, field: Field, lang: Lang) -> Vec<Choice> {
        let d = &self.draft;
        match field {
            Field::Interests | Field::ReadyDate => Vec::new(),
            Field::TargetRanks => RANKS
                .iter()
                .map(|r| Choice {
                    label: r.to_string(),
                    checked: d.target_ranks.iter().any(|t| t == r),
                    implied: false,
                })
                .collect(),
            Field::Categories => {
                let mut out = vec![Choice {
                    label: lang.pick("auto", "自动推断").to_string(),
                    checked: d.categories.is_empty(),
                    implied: false,
                }];
                out.extend(CATEGORIES.iter().map(|c| Choice {
                    label: c.sub.to_string(),
                    checked: d.categories.iter().any(|s| s == c.sub),
                    implied: false,
                }));
                out
            }
            Field::Horizon => {
                let mut out: Vec<Choice> = HORIZONS
                    .iter()
                    .map(|(days, en, zh)| Choice {
                        label: lang.pick(en, zh).to_string(),
                        checked: d.horizon_days == *days,
                        implied: false,
                    })
                    .collect();
                if !HORIZONS.iter().any(|(days, _, _)| *days == d.horizon_days) {
                    out.push(Choice {
                        label: match lang {
                            Lang::Zh => format!("{} 天", d.horizon_days),
                            Lang::En => format!("{} days", d.horizon_days),
                        },
                        checked: true,
                        implied: false,
                    });
                }
                out
            }
            Field::IncludeTbd => vec![Choice {
                label: lang
                    .pick("include unannounced deadlines", "包含截稿未公布的会议")
                    .to_string(),
                checked: d.include_tbd,
                implied: false,
            }],
            Field::PreferredPlaces | Field::AvoidedPlaces => {
                let places = if field == Field::PreferredPlaces {
                    &d.preferred_places
                } else {
                    &d.avoided_places
                };
                let checked: Vec<bool> = regions()
                    .iter()
                    .map(|region| region_selected(region, places))
                    .collect();
                regions()
                    .iter()
                    .enumerate()
                    .map(|(i, region)| {
                        let implied = checked[i]
                            && regions().iter().enumerate().any(|(j, other)| {
                                j != i
                                    && checked[j]
                                    && other.3.len() > region.3.len()
                                    && region.3.iter().all(|k| other.3.contains(k))
                            });
                        Choice {
                            label: lang.pick(region.1, region.0).to_string(),
                            checked: checked[i] && !implied,
                            implied,
                        }
                    })
                    .collect()
            }
        }
    }

    /// Places in the field that no region covers (typed in profile.yaml).
    pub fn custom_places(&self, field: Field) -> Vec<String> {
        let places = match field {
            Field::PreferredPlaces => &self.draft.preferred_places,
            Field::AvoidedPlaces => &self.draft.avoided_places,
            _ => return Vec::new(),
        };
        places
            .iter()
            .filter(|p| !regions().iter().any(|r| r.3.contains(&p.as_str())))
            .cloned()
            .collect()
    }

    fn option_count(&self, field: Field) -> usize {
        self.choices(field, Lang::En).len()
    }

    pub fn move_field(&mut self, delta: isize) {
        self.sync_text();
        let last = Field::ALL.len() as isize - 1;
        let index = (self.field.index() as isize + delta).clamp(0, last) as usize;
        self.focus(Field::ALL[index]);
    }

    pub fn focus(&mut self, field: Field) {
        self.sync_text();
        self.field = field;
        // Start on the first checked choice, like a radio group.
        self.option = self
            .choices(field, Lang::En)
            .iter()
            .position(|c| c.checked)
            .unwrap_or(0);
        if field.is_text() {
            self.text_mut().end();
        }
    }

    pub fn move_option(&mut self, delta: isize) {
        let count = self.option_count(self.field);
        if count == 0 {
            return;
        }
        self.option = (self.option as isize + delta).clamp(0, count as isize - 1) as usize;
    }

    fn text_mut(&mut self) -> &mut LineInput {
        if self.field == Field::ReadyDate {
            &mut self.ready
        } else {
            &mut self.interests
        }
    }

    /// Copy the text fields into the draft (on every edit).
    fn sync_text(&mut self) {
        self.draft.interests = split_list(&self.interests.text());
    }

    /// Space / click on a choice.
    pub fn toggle(&mut self, option: usize) {
        self.option = option;
        let d = &mut self.draft;
        match self.field {
            Field::Interests | Field::ReadyDate => {}
            Field::TargetRanks => {
                let rank = RANKS[option.min(RANKS.len() - 1)].to_string();
                if let Some(i) = d.target_ranks.iter().position(|r| *r == rank) {
                    // Keep at least one target rank.
                    if d.target_ranks.len() > 1 {
                        d.target_ranks.remove(i);
                    }
                } else {
                    d.target_ranks.push(rank);
                    d.target_ranks
                        .sort_by_key(|r| RANKS.iter().position(|x| x == r).unwrap_or(9));
                }
            }
            Field::Categories => {
                if option == 0 {
                    d.categories.clear();
                } else if let Some(c) = CATEGORIES.get(option - 1) {
                    let code = c.sub.to_string();
                    if let Some(i) = d.categories.iter().position(|s| *s == code) {
                        d.categories.remove(i);
                    } else {
                        d.categories.push(code);
                    }
                }
            }
            Field::Horizon => {
                if let Some((days, _, _)) = HORIZONS.get(option) {
                    d.horizon_days = *days;
                }
            }
            Field::IncludeTbd => d.include_tbd = !d.include_tbd,
            Field::PreferredPlaces | Field::AvoidedPlaces => {
                let Some(region) = regions().get(option) else {
                    return;
                };
                let avoid = self.field == Field::AvoidedPlaces;
                let (list, other) = if avoid {
                    (&mut d.avoided_places, &mut d.preferred_places)
                } else {
                    (&mut d.preferred_places, &mut d.avoided_places)
                };
                if region_selected(region, list) {
                    // Keep keywords still needed by another selected region.
                    let keep: Vec<&str> = regions()
                        .iter()
                        .filter(|r| r.0 != region.0 && region_selected(r, list))
                        .flat_map(|r| r.3.iter().copied())
                        .collect();
                    list.retain(|p| !region.3.contains(&p.as_str()) || keep.contains(&p.as_str()));
                } else {
                    for k in region.3 {
                        if !list.iter().any(|p| p == k) {
                            list.push(k.to_string());
                        }
                    }
                    // A place cannot be preferred and avoided at once.
                    other.retain(|p| !region.3.contains(&p.as_str()));
                }
            }
        }
    }

    /// The ready date as typed: `Ok(None)` when empty, `Err` when invalid.
    pub fn ready_date(&self, today: NaiveDate) -> Result<Option<NaiveDate>, InvalidDate> {
        let text = self.ready.text();
        if text.trim().is_empty() {
            return Ok(None);
        }
        parse_flexible_date(&text, today)
            .map(Some)
            .ok_or(InvalidDate)
    }

    /// Topics recognised in the interests, for the hint under the field.
    pub fn recognised(&self, lang: Lang) -> Vec<&'static str> {
        tags_for_text(&self.interests.text())
            .iter()
            .map(|t| tag_name(t, lang.is_zh()))
            .collect()
    }

    /// `周一，还有 123 天` / `Mon, in 123 days` for a valid date.
    pub fn date_hint(&self, today: NaiveDate, lang: Lang) -> Option<String> {
        let date = self.ready_date(today).ok()??;
        let days = (date - today).num_days();
        let weekday = date.weekday().num_days_from_monday() as usize;
        Some(match lang {
            Lang::Zh => {
                let names = ["周一", "周二", "周三", "周四", "周五", "周六", "周日"];
                let when = if days >= 0 {
                    format!("还有 {days} 天")
                } else {
                    format!("已过去 {} 天", -days)
                };
                format!("{date}，{}，{when}", names[weekday])
            }
            Lang::En => {
                let names = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
                let when = if days >= 0 {
                    format!("in {days} days")
                } else {
                    format!("{} days ago", -days)
                };
                format!("{date}, {}, {when}", names[weekday])
            }
        })
    }

    /// The profile to save, or an error message for the user.
    pub fn finish(&mut self, today: NaiveDate, lang: Lang) -> Result<Profile, String> {
        self.sync_text();
        match self.ready_date(today) {
            Ok(date) => {
                self.draft.ready_date = date;
                Ok(self.draft.clone())
            }
            Err(InvalidDate) => {
                self.focus(Field::ReadyDate);
                Err(lang
                    .pick(
                        "Paper ready: use a date such as 2027-03-01, 3月1日 or 3个月后.",
                        "论文完成时间无法识别：可写 2027-03-01、3月1日 或 3个月后。",
                    )
                    .to_string())
            }
        }
    }

    /// The draft as it would be saved now (for the preview).
    pub fn preview_profile(&self, today: NaiveDate) -> Profile {
        let mut profile = self.draft.clone();
        profile.interests = split_list(&self.interests.text());
        if let Ok(date) = self.ready_date(today) {
            profile.ready_date = date;
        }
        profile
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> FormOutcome {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        self.error = None;
        match key.code {
            KeyCode::Char('s') if ctrl => return FormOutcome::Save,
            KeyCode::Esc => return FormOutcome::Cancel,
            KeyCode::Up | KeyCode::BackTab => self.move_field(-1),
            KeyCode::Down | KeyCode::Tab => self.move_field(1),
            KeyCode::Enter => {
                if self.field == Field::AvoidedPlaces {
                    return FormOutcome::Save;
                }
                self.move_field(1);
            }
            _ if self.field.is_text() => {
                if self.text_mut().handle_key(key) {
                    self.sync_text();
                }
            }
            KeyCode::Left => self.move_option(-1),
            KeyCode::Right => self.move_option(1),
            KeyCode::Char(' ') => self.toggle(self.option),
            KeyCode::Home => self.option = 0,
            KeyCode::End => self.option = self.option_count(self.field).saturating_sub(1),
            _ => {}
        }
        FormOutcome::Continue
    }

    pub fn paste(&mut self, text: &str) {
        if self.field.is_text() {
            self.text_mut().insert_str(&text.replace(['\r', '\n'], " "));
            self.sync_text();
        }
    }

    /// The text field under edit, for the terminal cursor.
    pub fn active_text(&self) -> Option<&LineInput> {
        match self.field {
            Field::Interests => Some(&self.interests),
            Field::ReadyDate => Some(&self.ready),
            _ => None,
        }
    }

    /// Lines such as `目标等级：A, B → A` for every field that changed.
    pub fn changes(&self, today: NaiveDate, lang: Lang) -> Vec<String> {
        let before = &self.original;
        let after = self.preview_profile(today);
        let none = lang.pick("none", "无");
        let list = |items: &[String]| {
            if items.is_empty() {
                none.to_string()
            } else {
                items.join(", ")
            }
        };
        let date = |d: Option<NaiveDate>| d.map_or(none.to_string(), |d| d.to_string());
        let mut out = Vec::new();
        let mut push = |field: Field, a: String, b: String| {
            if a != b {
                out.push(format!("{}: {a} → {b}", field.label(lang)));
            }
        };
        push(
            Field::Interests,
            list(&before.interests),
            list(&after.interests),
        );
        push(
            Field::TargetRanks,
            list(&before.target_ranks),
            list(&after.target_ranks),
        );
        push(
            Field::Categories,
            list(&before.categories),
            list(&after.categories),
        );
        push(
            Field::ReadyDate,
            date(before.ready_date),
            date(after.ready_date),
        );
        push(
            Field::Horizon,
            before.horizon_days.to_string(),
            after.horizon_days.to_string(),
        );
        push(
            Field::IncludeTbd,
            before.include_tbd.to_string(),
            after.include_tbd.to_string(),
        );
        let places = |items: &[String]| {
            let names = place_names(items, lang);
            if names.is_empty() {
                none.to_string()
            } else {
                names.join(", ")
            }
        };
        push(
            Field::PreferredPlaces,
            places(&before.preferred_places),
            places(&after.preferred_places),
        );
        push(
            Field::AvoidedPlaces,
            places(&before.avoided_places),
            places(&after.avoided_places),
        );
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, 29).unwrap()
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn type_text(form: &mut ProfileForm, text: &str) {
        for c in text.chars() {
            form.handle_key(key(KeyCode::Char(c)));
        }
    }

    #[test]
    fn fields_can_be_visited_in_any_order() {
        let mut form = ProfileForm::new(&Profile::default());
        assert_eq!(form.field, Field::Interests);
        form.handle_key(key(KeyCode::Down));
        assert_eq!(form.field, Field::TargetRanks);
        form.handle_key(key(KeyCode::Enter));
        assert_eq!(form.field, Field::Categories);
        form.handle_key(key(KeyCode::Up));
        form.handle_key(key(KeyCode::Up));
        form.handle_key(key(KeyCode::Up));
        assert_eq!(form.field, Field::Interests);
        form.focus(Field::AvoidedPlaces);
        assert_eq!(form.handle_key(key(KeyCode::Enter)), FormOutcome::Save);
        assert_eq!(form.handle_key(key(KeyCode::Esc)), FormOutcome::Cancel);
        let save = KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL);
        assert_eq!(form.handle_key(save), FormOutcome::Save);
    }

    #[test]
    fn text_fields_edit_and_recognise_topics() {
        let mut form = ProfileForm::new(&Profile::default());
        type_text(&mut form, "多模态大模型, program analysis");
        assert_eq!(form.draft.interests, ["多模态大模型", "program analysis"]);
        assert!(!form.recognised(Lang::Zh).is_empty());
        // Space is text here, not a toggle.
        type_text(&mut form, " x");
        assert!(form.interests.text().ends_with(" x"));
        form.handle_key(key(KeyCode::Backspace));
        form.handle_key(key(KeyCode::Backspace));
        form.paste("\nsecurity\n");
        assert!(form.interests.text().ends_with(" security "));
    }

    #[test]
    fn ranks_categories_and_horizon_are_chosen_not_typed() {
        let mut form = ProfileForm::new(&Profile::default());
        form.focus(Field::TargetRanks);
        assert_eq!(form.option, 0); // A, the first checked one
        form.handle_key(key(KeyCode::Char(' '))); // A off
        assert_eq!(form.draft.target_ranks, ["B"]);
        form.handle_key(key(KeyCode::Char(' '))); // A on again
        form.handle_key(key(KeyCode::Right));
        form.handle_key(key(KeyCode::Char(' '))); // B off
        assert_eq!(form.draft.target_ranks, ["A"]);
        form.handle_key(key(KeyCode::Left));
        form.handle_key(key(KeyCode::Char(' '))); // the last rank stays
        assert_eq!(form.draft.target_ranks, ["A"]);

        form.focus(Field::Categories);
        assert!(form.choices(Field::Categories, Lang::En)[0].checked);
        form.toggle(8); // AI is the 8th category
        assert_eq!(form.draft.categories, [CATEGORIES[7].sub]);
        assert!(!form.choices(Field::Categories, Lang::En)[0].checked);
        form.toggle(0);
        assert!(form.draft.categories.is_empty());

        form.focus(Field::Horizon);
        assert_eq!(form.choices(Field::Horizon, Lang::En)[2].label, "1 year");
        form.toggle(1);
        assert_eq!(form.draft.horizon_days, 182);
        form.draft.horizon_days = 45;
        let custom = form.choices(Field::Horizon, Lang::Zh);
        assert_eq!(custom.last().unwrap().label, "45 天");

        form.focus(Field::IncludeTbd);
        form.handle_key(key(KeyCode::Char(' ')));
        assert!(!form.draft.include_tbd);
    }

    #[test]
    fn places_use_regions_and_keep_typed_extras() {
        let profile = Profile {
            preferred_places: vec!["Lisbon".into()],
            ..Profile::default()
        };
        let mut form = ProfileForm::new(&profile);
        form.focus(Field::PreferredPlaces);
        let china = regions().iter().position(|r| r.0 == "国内").unwrap();
        let asia = regions().iter().position(|r| r.0 == "亚洲").unwrap();
        form.toggle(china);
        assert!(form.draft.preferred_places.contains(&"China".to_string()));
        form.toggle(asia);
        assert!(form.draft.preferred_places.contains(&"Japan".to_string()));
        // Unticking 国内 keeps China for 亚洲.
        form.toggle(china);
        assert!(form.draft.preferred_places.contains(&"China".to_string()));
        assert!(!form.draft.preferred_places.contains(&"Macao".to_string()));
        assert_eq!(form.custom_places(Field::PreferredPlaces), ["Lisbon"]);
        let choices = form.choices(Field::PreferredPlaces, Lang::Zh);
        assert_eq!(choices[asia].label, "亚洲");
        assert!(choices[asia].checked);
        // Japan is part of Asia: shown as included, not as its own pick.
        let japan_choice = &choices[regions().iter().position(|r| r.0 == "日本").unwrap()];
        assert!(japan_choice.implied && !japan_choice.checked);

        // Avoiding Japan takes it out of the preferred places.
        form.focus(Field::AvoidedPlaces);
        let japan = regions().iter().position(|r| r.0 == "日本").unwrap();
        form.toggle(japan);
        assert_eq!(form.draft.avoided_places, ["Japan"]);
        assert!(!form.draft.preferred_places.contains(&"Japan".to_string()));
    }

    #[test]
    fn dates_are_validated_on_save() {
        let mut form = ProfileForm::new(&Profile::default());
        form.focus(Field::ReadyDate);
        type_text(&mut form, "3月1日");
        assert_eq!(
            form.date_hint(today(), Lang::Zh).unwrap(),
            "2027-03-01，周一，还有 123 天"
        );
        let profile = form.finish(today(), Lang::En).unwrap();
        assert_eq!(profile.ready_date, NaiveDate::from_ymd_opt(2027, 3, 1));

        form.ready.set("someday");
        form.focus(Field::Interests);
        assert!(form.finish(today(), Lang::Zh).is_err());
        assert_eq!(form.field, Field::ReadyDate);
        form.ready.clear();
        assert_eq!(form.finish(today(), Lang::En).unwrap().ready_date, None);
    }

    #[test]
    fn places_read_as_region_names() {
        let mut places: Vec<String> = regions()
            .iter()
            .find(|r| r.0 == "亚洲")
            .unwrap()
            .3
            .iter()
            .map(|s| s.to_string())
            .collect();
        places.push("Lisbon".into());
        assert_eq!(place_names(&places, Lang::Zh), ["亚洲", "Lisbon"]);
        assert_eq!(
            place_names(
                &[
                    "China".into(),
                    "Hong Kong".into(),
                    "Macau".into(),
                    "Macao".into()
                ],
                Lang::Zh
            ),
            ["国内"]
        );
    }

    #[test]
    fn changes_list_what_differs() {
        let mut form = ProfileForm::new(&Profile::default());
        assert!(form.changes(today(), Lang::En).is_empty());
        type_text(&mut form, "databases");
        form.focus(Field::TargetRanks);
        form.toggle(1);
        let changes = form.changes(today(), Lang::En);
        assert_eq!(changes[0], "Interests: none → databases");
        assert_eq!(changes[1], "Ranks: A, B → A");
    }
}
