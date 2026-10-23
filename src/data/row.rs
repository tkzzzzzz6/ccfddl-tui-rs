use chrono::{DateTime, Utc};

/// Display row for one conference (its best/next edition).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ConferenceRow {
    pub title: String,
    pub year: i32,
    pub sub: String,
    pub rank: String,
    pub core_rank: Option<String>,
    pub thcpl_rank: Option<String>,
    pub deadline: Option<DateTime<Utc>>,
    pub countdown: String,
    pub place: String,
    pub date: String,
    pub link: String,
    pub is_running: bool,
    pub is_tbd: bool,
    pub is_expired: bool,
    pub is_favorite: bool,
}

impl ConferenceRow {
    pub fn favorite_id(&self) -> String {
        super::favorites::favorite_id(&self.title, self.year)
    }
}
