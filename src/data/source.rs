//! Where conference data comes from: remote URL, on-disk cache, embedded snapshot.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_yaml_ng::Value;

use super::error::DataError;
use super::favorites::app_dir;
use crate::model::Conference;

pub const DEFAULT_URL: &str = "https://ccfddl.github.io/conference/allconf.yml";
pub const REMOTE_TIMEOUT: Duration = Duration::from_secs(30);

/// Snapshot compiled into the binary so the app works offline on first run.
pub const EMBEDDED_YAML: &str = include_str!("../../assets/allconf.yml");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataSource {
    Remote,
    Cache,
    Embedded,
}

impl DataSource {
    pub fn is_local(self) -> bool {
        !matches!(self, Self::Remote)
    }
}

pub fn default_cache_path() -> PathBuf {
    app_dir().join("allconf.yml")
}

pub fn parse_conferences(content: &str, origin: &str) -> Result<Vec<Conference>, DataError> {
    let data: Value = serde_yaml_ng::from_str(content)
        .map_err(|e| DataError::YamlParse(format!("{origin}: {e}")))?;
    match data {
        Value::Null => Err(DataError::YamlParse(format!("{origin}: empty YAML response"))),
        Value::Sequence(items) => Ok(items.iter().map(Conference::from_value).collect()),
        other => Err(DataError::Validation(format!(
            "{origin}: expected a list at the document root, got {}",
            value_kind(&other)
        ))),
    }
}

fn value_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Sequence(_) => "list",
        Value::Mapping(_) => "mapping",
        Value::Tagged(_) => "tagged value",
    }
}

pub fn fetch_remote(url: &str) -> Result<String, DataError> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(REMOTE_TIMEOUT))
        .build()
        .into();
    let mut response = agent
        .get(url)
        .call()
        .map_err(|e| DataError::Remote(format!("{url}: {e}")))?;
    response
        .body_mut()
        .read_to_string()
        .map_err(|e| DataError::Remote(format!("{url}: {e}")))
}

pub fn read_cache(path: &Path) -> Result<Vec<Conference>, DataError> {
    let content = fs::read_to_string(path)
        .map_err(|e| DataError::Local(format!("{}: {e}", path.display())))?;
    parse_conferences(&content, &path.display().to_string())
}

/// Best-effort cache write; failures only matter for logging.
pub fn write_cache(path: &Path, content: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("yml.tmp");
    fs::write(&tmp, content)?;
    fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    const SAMPLE: &str = "
- title: CVPR
  sub: AI
  rank: {ccf: A}
  confs:
  - year: 2025
    timezone: UTC
    timeline:
    - deadline: '2025-03-01 00:00:00'
- title: ICML
  sub: AI
  rank: {ccf: A}
  confs: []
";

    #[test]
    fn parses_list_root() {
        let confs = parse_conferences(SAMPLE, "test").unwrap();
        assert_eq!(confs.len(), 2);
        assert_eq!(confs[0].title, "CVPR");
        assert_eq!(confs[1].confs.len(), 0);
    }

    #[test]
    fn rejects_empty_and_non_list_documents() {
        assert!(matches!(parse_conferences("", "t"), Err(DataError::YamlParse(_))));
        assert!(matches!(parse_conferences("# only a comment", "t"), Err(DataError::YamlParse(_))));
        assert!(matches!(parse_conferences("title: x", "t"), Err(DataError::Validation(_))));
        assert!(matches!(parse_conferences("[unclosed", "t"), Err(DataError::YamlParse(_))));
    }

    #[test]
    fn cache_round_trip() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("sub").join("allconf.yml");
        write_cache(&path, SAMPLE).unwrap();
        assert_eq!(read_cache(&path).unwrap().len(), 2);
        assert!(!path.with_extension("yml.tmp").exists());
    }

    #[test]
    fn missing_cache_is_local_error() {
        let dir = tempdir().unwrap();
        assert!(matches!(read_cache(&dir.path().join("nope.yml")), Err(DataError::Local(_))));
    }

    #[test]
    fn embedded_snapshot_is_valid_and_complete() {
        let confs = parse_conferences(EMBEDDED_YAML, "embedded").unwrap();
        assert!(confs.len() > 200, "only {} conferences", confs.len());
        assert!(confs.iter().all(|c| !c.title.is_empty()));
        assert!(confs.iter().all(|c| crate::model::is_valid_sub(&c.sub)));
        assert!(confs.iter().any(|c| c.title == "CVPR"));
    }

    #[test]
    fn data_source_locality() {
        assert!(!DataSource::Remote.is_local());
        assert!(DataSource::Cache.is_local());
        assert!(DataSource::Embedded.is_local());
    }
}
