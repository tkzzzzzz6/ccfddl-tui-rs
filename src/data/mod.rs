//! Data layer: loading, processing, filtering, sorting and favorites.

pub mod error;
pub mod favorites;
pub mod process;
pub mod query;
pub mod row;
pub mod search;
pub mod service;
pub mod source;

pub use error::DataError;
pub use favorites::{Favorites, ToggleOutcome, favorite_id};
pub use query::FilterState;
pub use row::ConferenceRow;
pub use service::DataService;
pub use source::DataSource;
