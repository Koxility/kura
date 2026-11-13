use async_trait::async_trait;
use base64::{
    Engine,
    alphabet::STANDARD,
    engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig},
};
use kura_domain::Stream;

use crate::{Hoster, HosterError, Result, get};

/// VOE strips the padding from its base64.
const BASE64: GeneralPurpose = GeneralPurpose::new(
    &STANDARD,
    GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent),
);

/// Filler VOE scatters through the encoded player config.
const JUNK: [&str; 7] = ["@$", "^^", "~@", "%?", "*~", "!!", "#&"];

pub struct Voe {
    http: reqwest::Client,
}

impl Voe {
    pub fn new(http: reqwest::Client) -> Self {
        Self { http }
    }
}

#[async_trait]
impl Hoster for Voe {
    fn name(&self) -> &'static str {
        "voe"
    }

    async fn resolve(&self, url: &str) -> Result<Stream> {
        let mut html = get(&self.http, url).await?;
        // The public address only forwards to the current player domain, with JavaScript.
        if payload(&html).is_none()
            && let Some(player) = script_redirect(&html)
        {
            html = get(&self.http, player).await?;
        }
        let url = payload(&html)
            .and_then(decode)
            .ok_or(HosterError::NoStream("VOE"))?;
        Ok(Stream {
            url,
            headers: Vec::new(),
        })
    }
}

fn script_redirect(html: &str) -> Option<&str> {
    let (_, rest) = html.split_once("window.location.href = '")?;
    let (url, _) = rest.split_once('\'')?;
    url.starts_with("https://").then_some(url)
}

/// The encoded player config.
fn payload(html: &str) -> Option<&str> {
    let (_, rest) = html.split_once(r#"<script type="application/json">"#)?;
    Some(rest.split_once("</script>")?.0)
}

/// Undoes VOE's obfuscation and returns the HLS playlist URL from the player config.
fn decode(payload: &str) -> Option<String> {
    let encoded: [String; 1] = serde_json::from_str(payload.trim()).ok()?;
    let mut text: String = encoded[0].chars().map(rot13).collect();
    for junk in JUNK {
        text = text.replace(junk, "");
    }
    let mut bytes = BASE64.decode(text).ok()?;
    for byte in &mut bytes {
        *byte = byte.wrapping_sub(3);
    }
    bytes.reverse();
    let config: serde_json::Value = serde_json::from_slice(&BASE64.decode(bytes).ok()?).ok()?;
    let source = config.get("source")?.as_str()?;
    source.starts_with("https://").then(|| source.to_owned())
}

fn rot13(c: char) -> char {
    let rotate = |first: u8| ((c as u8 - first + 13) % 26 + first) as char;
    match c {
        'a'..='z' => rotate(b'a'),
        'A'..='Z' => rotate(b'A'),
        _ => c,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLAYER: &str = include_str!("../fixtures/voe_player.html");
    const REDIRECT: &str = include_str!("../fixtures/voe_redirect.html");

    #[test]
    fn player_page_decodes_to_the_playlist() {
        assert_eq!(
            payload(PLAYER).and_then(decode).as_deref(),
            Some("https://cdn.example/hls/abc123/master.m3u8?t=token")
        );
    }

    #[test]
    fn forwarding_page_points_to_the_player() {
        assert!(payload(REDIRECT).is_none());
        assert_eq!(
            script_redirect(REDIRECT),
            Some("https://player.example/e/abc123")
        );
    }

    #[test]
    fn garbage_is_not_a_stream() {
        assert_eq!(decode(r#"["not base64 at all"]"#), None);
        assert_eq!(decode("{}"), None);
        assert_eq!(payload("<html></html>"), None);
    }
}
