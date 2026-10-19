//! GIF search for the composer, backed by the GIPHY API.
//! Requests go through the backend so the API key never reaches the page,
//! and picked GIFs are downloaded here and embedded as data URIs (sent later
//! as inline cid: images by `smtp_sender`).

use anyhow::{Context, Result};
use base64::Engine;
use serde::{Deserialize, Serialize};

const MAX_GIF_BYTES: usize = 8 * 1024 * 1024;

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct GifDto {
    pub id: String,
    pub title: String,
    /// Small animated preview for the picker grid.
    pub preview_url: String,
    /// The rendition inserted into the email (200 px tall).
    pub url: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Deserialize)]
struct GiphyResponse {
    #[serde(default)]
    data: Vec<GiphyGif>,
}

#[derive(Deserialize)]
struct GiphyGif {
    id: String,
    #[serde(default)]
    title: String,
    images: GiphyImages,
}

#[derive(Deserialize)]
struct GiphyImages {
    fixed_height: Option<GiphyRendition>,
    fixed_height_small: Option<GiphyRendition>,
    fixed_width_small: Option<GiphyRendition>,
}

#[derive(Deserialize)]
struct GiphyRendition {
    url: String,
    #[serde(default)]
    width: String,
    #[serde(default)]
    height: String,
}

/// Turn a GIPHY search/trending response into picker entries.
pub fn parse_giphy(json: &str) -> Result<Vec<GifDto>> {
    let resp: GiphyResponse = serde_json::from_str(json).context("Unexpected GIPHY response")?;
    Ok(resp
        .data
        .into_iter()
        .filter_map(|g| {
            let full = g.images.fixed_height?;
            let preview = g
                .images
                .fixed_width_small
                .or(g.images.fixed_height_small)
                .map(|r| r.url)
                .unwrap_or_else(|| full.url.clone());
            Some(GifDto {
                id: g.id,
                title: g.title,
                preview_url: preview,
                width: full.width.parse().unwrap_or(0),
                height: full.height.parse().unwrap_or(0),
                url: full.url,
            })
        })
        .collect())
}

/// Search GIPHY; an empty query returns trending GIFs.
pub async fn search(api_key: &str, query: &str) -> Result<Vec<GifDto>> {
    if api_key.trim().is_empty() {
        anyhow::bail!("Add a GIPHY API key in Settings → AI to search GIFs");
    }
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()?;
    let query = query.trim();
    let req = if query.is_empty() {
        client
            .get("https://api.giphy.com/v1/gifs/trending")
            .query(&[("api_key", api_key), ("limit", "24"), ("rating", "g")])
    } else {
        client
            .get("https://api.giphy.com/v1/gifs/search")
            .query(&[("api_key", api_key), ("q", query), ("limit", "24"), ("rating", "g")])
    };
    let resp = req.send().await.context("Could not reach GIPHY")?;
    let status = resp.status();
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        anyhow::bail!("GIPHY rejected the API key — check it in Settings → AI");
    }
    if !status.is_success() {
        anyhow::bail!("GIPHY error {}", status);
    }
    parse_giphy(&resp.text().await?)
}

/// Only GIPHY's media hosts may be downloaded through the backend.
pub fn is_giphy_media_url(url: &str) -> bool {
    match reqwest::Url::parse(url) {
        Ok(u) => {
            u.scheme() == "https"
                && u.host_str().map_or(false, |h| h == "giphy.com" || h.ends_with(".giphy.com"))
        }
        Err(_) => false,
    }
}

/// Download a GIF and return it as a `data:image/gif;base64,…` URI.
pub async fn fetch_as_data_uri(url: &str) -> Result<String> {
    if !is_giphy_media_url(url) {
        anyhow::bail!("Not a GIPHY media URL");
    }
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()?;
    let resp = client.get(url).send().await.context("Could not download the GIF")?;
    if !resp.status().is_success() {
        anyhow::bail!("GIF download failed ({})", resp.status());
    }
    let bytes = resp.bytes().await?;
    if bytes.len() > MAX_GIF_BYTES {
        anyhow::bail!("This GIF is too large to send (over 8 MB)");
    }
    Ok(format!(
        "data:image/gif;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(&bytes)
    ))
}
