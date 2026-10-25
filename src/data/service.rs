//! `DataService`: ties sources, favorites and processing together.
//! Fallback order is remote -> on-disk cache -> embedded snapshot.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};

use super::error::DataError;
use super::favorites::{Favorites, ToggleOutcome};
use super::process::process_rows;
use super::row::ConferenceRow;
use super::source::{
    DEFAULT_URL, DataSource, EMBEDDED_YAML, default_cache_path, fetch_remote, parse_conferences,
    read_cache, write_cache,
};
use crate::model::{CATEGORIES, Conference};

pub struct DataService {
    url: String,
    cache_path: PathBuf,
    conferences: Vec<Conference>,
    source: Option<DataSource>,
    last_error: Option<String>,
    favorites: Favorites,
}

impl DataService {
    pub const DEFAULT_URL: &'static str = DEFAULT_URL;

    pub fn new(url: Option<String>) -> Self {
        Self::with_paths(url, default_cache_path(), Favorites::load_default())
    }

    pub fn with_paths(url: Option<String>, cache_path: PathBuf, favorites: Favorites) -> Self {
        Self {
            url: url.unwrap_or_else(|| DEFAULT_URL.to_string()),
            cache_path,
            conferences: Vec::new(),
            source: None,
            last_error: None,
            favorites,
        }
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn conferences(&self) -> &[Conference] {
        &self.conferences
    }

    pub fn source(&self) -> Option<DataSource> {
        self.source
    }

    pub fn using_local_data(&self) -> bool {
        self.source.is_some_and(DataSource::is_local)
    }

    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    pub fn favorites(&self) -> &Favorites {
        &self.favorites
    }

    pub fn cache_path(&self) -> &Path {
        &self.cache_path
    }

    /// Preserve the caller's in-memory favorites when replacing a service
    /// after a background refresh.
    pub(crate) fn set_favorites(&mut self, favorites: Favorites) {
        self.favorites = favorites;
    }

    pub fn is_favorite(&self, title: &str, year: i32) -> bool {
        self.favorites.contains(title, year)
    }

    pub fn toggle_favorite(&mut self, row: &ConferenceRow) -> ToggleOutcome {
        self.favorites.toggle(&row.title, row.year)
    }

    /// Load conferences from the configured URL, with fallbacks.
    pub fn load_conferences(&mut self) -> Result<&[Conference], DataError> {
        let url = self.url.clone();
        self.load_conferences_with(move || fetch_remote(&url))
    }

    /// Same as [`load_conferences`](Self::load_conferences) with an injectable fetcher (for tests).
    pub fn load_conferences_with<F>(&mut self, fetch: F) -> Result<&[Conference], DataError>
    where
        F: FnOnce() -> Result<String, DataError>,
    {
        self.last_error = None;

        let remote_error = match fetch()
            .and_then(|body| parse_conferences(&body, &self.url).map(|confs| (body, confs)))
        {
            Ok((body, confs)) => {
                if let Err(e) = write_cache(&self.cache_path, &body) {
                    log::warn!("Failed to write data cache: {e}");
                }
                return Ok(self.finish(confs, DataSource::Remote));
            }
            Err(e) => e,
        };
        log::warn!("Failed to load remote data: {remote_error}");
        self.last_error = Some(format!("Remote: {remote_error}"));

        let cache_error = match read_cache(&self.cache_path) {
            Ok(confs) => return Ok(self.finish(confs, DataSource::Cache)),
            Err(e) => e,
        };

        match parse_conferences(EMBEDDED_YAML, "embedded snapshot") {
            Ok(confs) => Ok(self.finish(confs, DataSource::Embedded)),
            Err(embedded_error) => Err(DataError::AllSourcesFailed(format!(
                "Remote: {remote_error}; Cache: {cache_error}; Embedded: {embedded_error}"
            ))),
        }
    }

    fn finish(&mut self, confs: Vec<Conference>, source: DataSource) -> &[Conference] {
        self.conferences = confs;
        self.source = Some(source);
        &self.conferences
    }

    pub fn process_rows(&self, now: DateTime<Utc>) -> Vec<ConferenceRow> {
        process_rows(&self.conferences, now, &self.favorites)
    }

    pub fn get_category_name(&self, sub: &str) -> String {
        CATEGORIES
            .iter()
            .find(|c| c.sub == sub)
            .map_or_else(|| sub.to_string(), |c| c.name_en.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use std::fs;
    use tempfile::{TempDir, tempdir};

    const REMOTE_YAML: &str = "
- title: RemoteConf
  sub: AI
  rank: {ccf: A}
  confs:
  - year: 2030
    timezone: UTC
    timeline:
    - deadline: '2030-03-01 00:00:00'
";

    fn service(dir: &TempDir) -> DataService {
        DataService::with_paths(
            Some("https://example.invalid/allconf.yml".into()),
            dir.path().join("allconf.yml"),
            Favorites::empty_at(dir.path().join("favorites.json")),
        )
    }

    fn net_down() -> Result<String, DataError> {
        Err(DataError::Remote("connection refused".into()))
    }

    #[test]
    fn default_and_custom_url() {
        assert_eq!(
            DataService::DEFAULT_URL,
            "https://ccfddl.github.io/conference/allconf.yml"
        );
        let dir = tempdir().unwrap();
        assert_eq!(service(&dir).url(), "https://example.invalid/allconf.yml");
        let default = DataService::with_paths(
            None,
            dir.path().join("c.yml"),
            Favorites::empty_at(dir.path().join("f.json")),
        );
        assert_eq!(default.url(), DEFAULT_URL);
    }

    #[test]
    fn remote_success_updates_cache() {
        let dir = tempdir().unwrap();
        let mut svc = service(&dir);
        let confs = svc
            .load_conferences_with(|| Ok(REMOTE_YAML.to_string()))
            .unwrap();
        assert_eq!(confs.len(), 1);
        assert_eq!(svc.source(), Some(DataSource::Remote));
        assert!(!svc.using_local_data());
        assert_eq!(svc.last_error(), None);
        assert_eq!(
            fs::read_to_string(dir.path().join("allconf.yml")).unwrap(),
            REMOTE_YAML
        );
    }

    #[test]
    fn network_error_falls_back_to_cache() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("allconf.yml"), REMOTE_YAML).unwrap();
        let mut svc = service(&dir);

        let confs = svc.load_conferences_with(net_down).unwrap();
        assert_eq!(confs[0].title, "RemoteConf");
        assert_eq!(svc.source(), Some(DataSource::Cache));
        assert!(svc.using_local_data());
        assert!(svc.last_error().unwrap().contains("connection refused"));
    }

    #[test]
    fn no_cache_falls_back_to_embedded_snapshot() {
        let dir = tempdir().unwrap();
        let mut svc = service(&dir);
        let confs = svc.load_conferences_with(net_down).unwrap();
        assert!(confs.len() > 200);
        assert_eq!(svc.source(), Some(DataSource::Embedded));
        assert!(svc.using_local_data());
    }

    #[test]
    fn invalid_remote_body_falls_back_and_does_not_poison_cache() {
        let dir = tempdir().unwrap();
        let mut svc = service(&dir);
        for bad in ["", "title: not-a-list", "[broken"] {
            svc.load_conferences_with(|| Ok(bad.to_string())).unwrap();
            assert_eq!(svc.source(), Some(DataSource::Embedded), "{bad:?}");
            assert!(svc.last_error().is_some());
        }
        assert!(!dir.path().join("allconf.yml").exists());
    }

    #[test]
    fn corrupt_cache_falls_through_to_embedded() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("allconf.yml"), "[broken").unwrap();
        let mut svc = service(&dir);
        svc.load_conferences_with(net_down).unwrap();
        assert_eq!(svc.source(), Some(DataSource::Embedded));
    }

    #[test]
    fn unwritable_cache_does_not_fail_load() {
        let dir = tempdir().unwrap();
        let blocker = dir.path().join("blocker");
        fs::write(&blocker, "file").unwrap();
        let mut svc = DataService::with_paths(
            None,
            blocker.join("allconf.yml"),
            Favorites::empty_at(dir.path().join("f.json")),
        );
        svc.load_conferences_with(|| Ok(REMOTE_YAML.to_string()))
            .unwrap();
        assert_eq!(svc.source(), Some(DataSource::Remote));
    }

    #[test]
    fn process_rows_uses_loaded_data_and_favorites() {
        let dir = tempdir().unwrap();
        let mut svc = service(&dir);
        svc.load_conferences_with(|| Ok(REMOTE_YAML.to_string()))
            .unwrap();
        let now = Utc.with_ymd_and_hms(2025, 1, 15, 12, 0, 0).unwrap();

        let rows = svc.process_rows(now);
        assert_eq!(rows.len(), 1);
        assert!(!rows[0].is_favorite);

        let outcome = svc.toggle_favorite(&rows[0]);
        assert!(outcome.is_favorite);
        assert!(svc.is_favorite("RemoteConf", 2030));
        assert!(svc.process_rows(now)[0].is_favorite);
        assert!(Favorites::load(dir.path().join("favorites.json")).contains("RemoteConf", 2030));
    }

    #[test]
    fn process_rows_before_load_is_empty() {
        let dir = tempdir().unwrap();
        assert!(service(&dir).process_rows(Utc::now()).is_empty());
    }

    #[test]
    fn category_names() {
        let dir = tempdir().unwrap();
        let svc = service(&dir);
        assert_eq!(svc.get_category_name("AI"), "Artificial Intelligence");
        assert_eq!(svc.get_category_name("DB"), "Database");
        assert_eq!(svc.get_category_name("ZZ"), "ZZ");
    }

    #[test]
    fn full_embedded_pipeline_produces_sane_rows() {
        let dir = tempdir().unwrap();
        let mut svc = service(&dir);
        svc.load_conferences_with(net_down).unwrap();
        let now = Utc.with_ymd_and_hms(2025, 6, 1, 0, 0, 0).unwrap();

        let rows = svc.process_rows(now);
        assert!(rows.len() > 200);
        assert!(
            rows.iter()
                .all(|r| r.is_running as u8 + r.is_expired as u8 == 1)
        );
        assert!(rows.iter().any(|r| r.is_tbd));
        assert!(
            rows.iter()
                .filter(|r| r.deadline.is_some_and(|d| d > now))
                .all(|r| r.is_running)
        );
    }
}
