//! M2 go/no-go spike (PLAN.md §6): prove that a pure-Rust InnerTube client can
//! do what the yt-dlp binary does for Nuclear — search videos and resolve a
//! playable audio stream URL — with no process spawning (Android W^X safe).
//!
//! Success criteria:
//!   1. search returns results for a query        (≈ ytdlp_search)
//!   2. a video id resolves to audio streams      (≈ ytdlp_get_stream)
//!   3. the resolved URL actually serves bytes    (HTTP 200/206, audio/* or video/*)
//!
//! Run: cargo run  (exit 0 = GO, exit 1 = NO-GO with the failure printed)

use anyhow::{bail, Context, Result};
use rustypipe::client::RustyPipe;
use rustypipe::model::VideoItem;

#[tokio::main]
async fn main() -> Result<()> {
    let query = std::env::args().nth(1).unwrap_or_else(|| "never gonna give you up".into());

    let rp = RustyPipe::new();

    // 1) Search (≈ ytdlp_search)
    let search = rp
        .query()
        .search::<VideoItem, _>(&query)
        .await
        .context("search failed")?;
    let items = &search.items.items;
    if items.is_empty() {
        bail!("NO-GO: search returned zero results");
    }
    println!("[1/3] search OK: {} results", items.len());
    for v in items.iter().take(3) {
        println!("      {} - {} ({}s)", v.id, v.name, v.duration.unwrap_or(0));
    }

    // 2) Resolve streams (≈ ytdlp_get_stream)
    let video_id = &items[0].id;
    let player = rp
        .query()
        .player(video_id)
        .await
        .context("player (stream resolution) failed")?;
    if player.audio_streams.is_empty() {
        bail!("NO-GO: no audio streams resolved for {video_id}");
    }
    // Prefer m4a like ytdlp_get_stream does, else highest-bitrate anything.
    let best = player
        .audio_streams
        .iter()
        .filter(|s| s.mime.contains("mp4"))
        .max_by_key(|s| s.bitrate)
        .or_else(|| player.audio_streams.iter().max_by_key(|s| s.bitrate))
        .unwrap();
    println!(
        "[2/3] resolve OK: {} audio streams; best: itag={} {} {}bps",
        player.audio_streams.len(),
        best.itag,
        best.mime,
        best.bitrate
    );

    // 3) Fetch bytes (proves the URL is actually playable, not just present)
    let client = reqwest::Client::new();
    let resp = client
        .get(&best.url)
        .header("Range", "bytes=0-65535")
        .send()
        .await
        .context("stream URL fetch failed")?;
    let status = resp.status();
    let ctype = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("?")
        .to_string();
    let bytes = resp.bytes().await.context("reading stream body failed")?;
    if !(status.is_success() || status.as_u16() == 206) || bytes.is_empty() {
        bail!("NO-GO: stream URL returned {status} with {} bytes", bytes.len());
    }
    println!("[3/3] fetch OK: HTTP {status}, content-type {ctype}, {} bytes read", bytes.len());
    println!("GO: rustypipe can replace yt-dlp for search + stream resolution");
    Ok(())
}
