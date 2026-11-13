//! The two things Kura asks the outside world besides sites and hosters: its public address
//! and whether a newer release exists.

use std::{
    net::IpAddr,
    sync::{Arc, PoisonError},
    time::{Duration, Instant},
};

use serde::Deserialize;

use crate::{Core, CoreError, Result};

/// The GitHub repository, as `owner/name`.
pub const REPOSITORY: &str = "Koxility/kura";

/// GitHub is asked at most this often.
const UPDATE_CHECK_EVERY: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Deserialize)]
struct Release {
    tag_name: String,
}

impl Core {
    /// The address the server reaches the internet from, e.g. to check that a VPN is in use.
    /// Asks an outside service, so it only runs when a person presses the button.
    pub async fn public_ip(&self) -> Result<String> {
        let http = kura_sites::http_client()?;
        let answer = http.get("https://api.ipify.org").send().await?;
        let text = answer.error_for_status()?.text().await?;
        // Whatever the service sends is shown to the admin, so it has to be an address.
        let address: IpAddr = text.trim().parse().map_err(|_| {
            CoreError::Invalid("the address service sent something that is not an address")
        })?;
        Ok(address.to_string())
    }

    /// The newest release on GitHub, without the leading `v`, when it is newer than this
    /// build. Answers from memory at once and refreshes in the background at most once a day,
    /// so no page waits for GitHub.
    pub fn newer_release(self: &Arc<Self>) -> Option<String> {
        let mut update = self.update.lock().unwrap_or_else(PoisonError::into_inner);
        let (stale, known) = match &*update {
            Some((checked, known)) => (checked.elapsed() > UPDATE_CHECK_EVERY, known.clone()),
            None => (true, None),
        };
        if stale {
            // Marked as checked first, so a burst of page loads starts one request.
            *update = Some((Instant::now(), known.clone()));
            let core = self.clone();
            tokio::spawn(async move {
                let newer = latest_release()
                    .await
                    .filter(|tag| is_newer(tag, env!("CARGO_PKG_VERSION")))
                    .map(|tag| tag.trim_start_matches('v').to_owned());
                let mut update = core.update.lock().unwrap_or_else(PoisonError::into_inner);
                *update = Some((Instant::now(), newer));
            });
        }
        known
    }
}

/// The tag of the latest release; `None` when there is none or GitHub does not answer.
async fn latest_release() -> Option<String> {
    let url = format!("https://api.github.com/repos/{REPOSITORY}/releases/latest");
    let http = kura_sites::http_client().ok()?;
    let answer = http.get(url).send().await.ok()?.error_for_status().ok()?;
    Some(answer.json::<Release>().await.ok()?.tag_name)
}

/// Compares versions like `v0.2.0` and `0.1.9` number by number.
fn is_newer(latest: &str, current: &str) -> bool {
    let numbers = |version: &str| -> Option<Vec<u32>> {
        let version = version.trim().trim_start_matches('v');
        version.split('.').map(|part| part.parse().ok()).collect()
    };
    match (numbers(latest), numbers(current)) {
        (Some(latest), Some(current)) => latest > current,
        // A tag that is not a version says nothing.
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_by_number() {
        assert!(is_newer("v0.2.0", "0.1.9"));
        assert!(is_newer("0.10.0", "0.9.5"));
        assert!(is_newer("1.0", "0.9.9"));
        assert!(!is_newer("v0.1.0", "0.1.0"));
        assert!(!is_newer("0.0.9", "0.1.0"));
        assert!(!is_newer("nightly", "0.1.0"));
        assert!(!is_newer("v1.0.0-beta", "0.1.0"));
    }
}
