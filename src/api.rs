use std::time::Duration;

use anyhow::{bail, Context};
use embedded_svc::http::client::Client;
use esp_idf_svc::http::client::{Configuration, EspHttpConnection};
use log::{debug, info};
use serde::Deserialize;

use crate::state::TalkData;

#[derive(Debug, Clone, Deserialize)]
struct Talk {
    pub title: String,
    pub titles: Vec<String>,
    #[serde(default)]
    pub step_counts: Vec<usize>,
}

impl From<Talk> for TalkData {
    fn from(talk: Talk) -> Self {
        // If step_counts is empty or shorter than titles, fill with 0s
        let step_counts = if talk.step_counts.is_empty() {
            vec![0; talk.titles.len()]
        } else {
            let mut counts = talk.step_counts;
            counts.resize(talk.titles.len(), 0);
            counts
        };
        TalkData::new(talk.title, talk.titles, step_counts)
    }
}

pub struct Api {
    client: Client<EspHttpConnection>,
    base_url: String,
}

impl Api {
    pub(crate) fn new(base_url: String) -> anyhow::Result<Self> {
        let configuration = Configuration {
            timeout: Some(Duration::from_secs(60)),
            buffer_size: Some(2048),    // Reduced buffer size
            buffer_size_tx: Some(1024), // Reduced TX buffer size
            use_global_ca_store: false, // Explicitly disable TLS for HTTP
            crt_bundle_attach: None,    // No certificate bundle for HTTP
            ..Default::default()
        };
        let conn =
            EspHttpConnection::new(&configuration).context("creating the HTTP connection")?;
        let client = Client::wrap(conn);
        Ok(Self { client, base_url })
    }

    pub(crate) fn talk(&mut self) -> anyhow::Result<TalkData> {
        let uri = format!("{}/api/talk", self.base_url.trim_end_matches('/'));
        info!("🌐 Attempting HTTP GET: {uri}");

        let request = self
            .client
            .get(&uri)
            .with_context(|| format!("build GET {uri} request"))?;

        info!("🌐 Request built successfully, submitting...");
        let mut response = request.submit().with_context(|| format!("GET {uri}"))?;
        let status = response.status();
        info!("Status: [{status}]",);

        let mut result = vec![];
        let mut buf = [0_u8; 256];
        loop {
            // A read error used to end this loop silently, so a truncated body
            // resurfaced as a bewildering JSON parse failure.
            let size = response
                .read(&mut buf)
                .with_context(|| format!("read the {uri} body after {} bytes", result.len()))?;
            if size == 0 {
                break;
            }
            result.extend(buf.get(0..size).unwrap_or(&[]));
        }
        debug!("total len: {}", result.len());

        let body = String::from_utf8_lossy(&result);
        debug!("JSON response: {body}");

        if !(200..300).contains(&status) {
            bail!("HTTP {status}: {}", snippet(&body));
        }

        let talk = serde_json::from_str::<Talk>(&body)
            .with_context(|| format!("parse the talk from {}", snippet(&body)))?;

        Ok(talk.into())
    }
}

/// Trims a response body down to something a 320x240 screen can carry.
fn snippet(body: &str) -> String {
    const MAX: usize = 80;

    let trimmed = body.trim();
    if trimmed.is_empty() {
        return "<empty body>".to_string();
    }
    match trimmed.char_indices().nth(MAX) {
        Some((index, _)) => format!("{}...", &trimmed[..index]),
        None => trimmed.to_string(),
    }
}
