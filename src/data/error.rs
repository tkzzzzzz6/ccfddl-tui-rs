use thiserror::Error;

#[derive(Debug, Error)]
pub enum DataError {
    #[error("[remote] {0}")]
    Remote(String),
    #[error("[yaml] {0}")]
    YamlParse(String),
    #[error("[validation] {0}")]
    Validation(String),
    #[error("[local] {0}")]
    Local(String),
    #[error("Failed to load conference data from any source: {0}")]
    AllSourcesFailed(String),
}
