//! Background conference loading and refresh.
//!
//! The network request runs on a worker thread. The UI keeps rendering and
//! processing input, then applies the returned `DataService` on the next event
//! loop iteration.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::thread;

use chrono::Utc;

use crate::data::source::fetch_remote;
use crate::data::{DataError, DataService, Favorites};

pub struct RefreshRequest {
    pub url: String,
    pub cache_path: PathBuf,
    pub favorites_path: PathBuf,
}

impl RefreshRequest {
    pub fn from_service(service: &DataService) -> Self {
        Self {
            url: service.url().to_string(),
            cache_path: service.cache_path().to_path_buf(),
            favorites_path: service.favorites().path().to_path_buf(),
        }
    }
}

pub struct RefreshMessage {
    pub service: DataService,
    pub result: Result<(), String>,
}

/// Start a real network refresh in the background.
pub fn spawn(service: &DataService) -> Receiver<RefreshMessage> {
    let request = RefreshRequest::from_service(service);
    let url = request.url.clone();
    spawn_with(request, move || fetch_remote(&url))
}

/// Injectable fetcher used by tests and by the production network path.
fn spawn_with<F>(request: RefreshRequest, fetch: F) -> Receiver<RefreshMessage>
where
    F: FnOnce() -> Result<String, DataError> + Send + 'static,
{
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let mut service = DataService::with_paths(
            Some(request.url),
            request.cache_path,
            Favorites::load(request.favorites_path),
        );
        let result = match service.load_conferences_with(fetch) {
            Ok(_) => {
                // Exercise row construction on the worker as well, so parse or
                // timezone failures never reach the UI thread unexpectedly.
                let _ = service.process_rows(Utc::now());
                Ok(())
            }
            Err(error) => Err(error.to_string()),
        };
        let _ = sender.send(RefreshMessage { service, result });
    });
    receiver
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::TryRecvError;
    use std::time::Duration;
    use tempfile::TempDir;

    const YAML: &str = "
- title: RemoteConf
  sub: AI
  rank: {ccf: A}
  confs:
  - year: 2030
    timezone: UTC
    timeline:
    - deadline: '2030-03-01 00:00:00'
";

    fn request(dir: &TempDir) -> RefreshRequest {
        RefreshRequest {
            url: "https://example.invalid/allconf.yml".to_string(),
            cache_path: dir.path().join("cache.yml"),
            favorites_path: dir.path().join("favorites.json"),
        }
    }

    #[test]
    fn refresh_returns_loaded_service() {
        let dir = tempfile::tempdir().unwrap();
        let receiver = spawn_with(request(&dir), || Ok(YAML.to_string()));
        let message = receiver.recv_timeout(Duration::from_secs(2)).unwrap();

        assert!(message.result.is_ok());
        assert_eq!(message.service.conferences().len(), 1);
        assert_eq!(message.service.conferences()[0].title, "RemoteConf");
    }

    #[test]
    fn waiting_fetch_does_not_complete_early() {
        let dir = tempfile::tempdir().unwrap();
        let (gate_sender, gate_receiver) = mpsc::channel();
        let receiver = spawn_with(request(&dir), move || {
            gate_receiver.recv().unwrap();
            Ok(YAML.to_string())
        });

        // The worker is blocked in the fetcher, but the caller is free to run.
        assert!(matches!(receiver.try_recv(), Err(TryRecvError::Empty)));
        gate_sender.send(()).unwrap();
        assert!(
            receiver
                .recv_timeout(Duration::from_secs(2))
                .unwrap()
                .result
                .is_ok()
        );
    }

    #[test]
    fn malformed_remote_data_falls_back_safely() {
        let dir = tempfile::tempdir().unwrap();
        let receiver = spawn_with(request(&dir), || Ok("not: a list".to_string()));
        let message = receiver.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(message.result.is_ok());
        assert!(message.service.using_local_data());
        assert!(message.service.last_error().is_some());
    }
}
