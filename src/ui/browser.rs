//! Opening a conference URL in the default browser.
//!
//! The Python app calls `webbrowser.open`. Rust has no equivalent in the
//! standard library, and the `open`/`webbrowser` crates would be a new
//! dependency, so this shells out to the platform opener directly.
//!
//! `std::process::Command` does not go through a shell, so a URL can never be
//! interpreted as a command — important because URLs are downloaded data.

use std::io;
use std::process::{Command, Stdio};

/// Whether a string is an `http`/`https` URL we are willing to open.
///
/// Mirrors `URL_PATTERN = re.compile(r"^https?://")` in the Python table: the
/// check is deliberately narrow so that `file://`, `javascript:` and similar
/// schemes can never be handed to the platform opener.
pub fn is_openable_url(url: &str) -> bool {
    url.starts_with("http://") || url.starts_with("https://")
}

/// Open `url` in the user's default browser.
///
/// Returns an error for anything that is not an http(s) URL, and for platforms
/// where the opener is unavailable.
pub fn open_url(url: &str) -> io::Result<()> {
    if !is_openable_url(url) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("Refusing to open non-http URL: {url}"),
        ));
    }
    open_in_default_browser(url)
}

/// The platform-specific part of [`open_url`], split out so the URL policy can
/// be tested without spawning anything.
#[cfg(target_os = "windows")]
fn open_in_default_browser(url: &str) -> io::Result<()> {
    // `rundll32 url.dll,FileProtocolHandler` hands the URL to the default
    // handler without a shell. `cmd /c start` would parse `&` and friends, and
    // `explorer.exe` flashes a transient window.
    Command::new("rundll32")
        .arg("url.dll,FileProtocolHandler")
        .arg(url)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
}

#[cfg(target_os = "macos")]
fn open_in_default_browser(url: &str) -> io::Result<()> {
    Command::new("open")
        .arg(url)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn open_in_default_browser(url: &str) -> io::Result<()> {
    Command::new("xdg-open")
        .arg(url)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_http_and_https() {
        assert!(is_openable_url("https://cvpr.thecvf.com/"));
        assert!(is_openable_url("http://example.com"));
        assert!(is_openable_url("http://localhost:8080/path?a=1&b=2"));
    }

    #[test]
    fn rejects_other_schemes_and_junk() {
        for bad in [
            "",
            "   ",
            "ftp://example.com",
            "file:///etc/passwd",
            "javascript:alert(1)",
            "data:text/html,<script>",
            "example.com",
            " //example.com",
            "HTTPS://EXAMPLE.COM",
        ] {
            assert!(!is_openable_url(bad), "should reject {bad:?}");
        }
    }

    #[test]
    fn open_url_refuses_non_http_urls() {
        let err = open_url("file:///etc/passwd").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn urls_with_shell_metacharacters_are_still_opened() {
        // These are valid URLs. No shell is involved, so `&` and spaces are
        // passed through as data rather than interpreted.
        assert!(is_openable_url("https://example.com/?a=1&b=2"));
        assert!(is_openable_url("https://example.com/a b"));
    }
}
