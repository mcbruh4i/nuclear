//! Mobile backend for the Ytdlp command surface.
//!
//! The yt-dlp binary cannot run on Android/iOS (W^X forbids executing downloaded
//! binaries since API 29), so on mobile the `ytdlp_search` / `ytdlp_get_stream` /
//! `ytdlp_get_playlist` commands are backed by RustyPipe, a pure-Rust InnerTube
//! client - same JS-visible contract, no process spawning. Validated by
//! `spikes/m2-rustypipe` (search -> resolve -> HTTP 206 audio bytes).

use std::sync::OnceLock;

use log::debug;
use rustypipe::client::RustyPipe;
use rustypipe::model::VideoItem;

use crate::ytdlp::{
    YtdlpPlaylistEntry, YtdlpPlaylistInfo, YtdlpSearchResult, YtdlpStreamInfo, YtdlpThumbnail,
};

static CLIENT: OnceLock<RustyPipe> = OnceLock::new();
static STORAGE_DIR: OnceLock<std::path::PathBuf> = OnceLock::new();

/// Called once from `lib.rs` setup. RustyPipe caches client versions and the
/// stream-URL deobfuscation code on disk; on mobile the only writable place is
/// the app data dir.
pub fn init(app_handle: &tauri::AppHandle) {
    use tauri::Manager;
    if let Ok(dir) = app_handle.path().app_data_dir() {
        let _ = STORAGE_DIR.set(dir.join("rustypipe"));
    }
}

fn client() -> Result<&'static RustyPipe, String> {
    if let Some(client) = CLIENT.get() {
        return Ok(client);
    }
    let mut builder = RustyPipe::builder();
    if let Some(dir) = STORAGE_DIR.get() {
        std::fs::create_dir_all(dir)
            .map_err(|error| format!("Failed to create rustypipe storage dir: {error}"))?;
        builder = builder.storage_dir(dir.clone());
    }
    let built = builder
        .build()
        .map_err(|error| format!("Failed to initialize RustyPipe: {error}"))?;
    Ok(CLIENT.get_or_init(|| built))
}

fn best_thumbnail_url(thumbnails: &[rustypipe::model::Thumbnail]) -> Option<String> {
    thumbnails
        .iter()
        .max_by_key(|thumbnail| thumbnail.width * thumbnail.height)
        .map(|thumbnail| thumbnail.url.clone())
}

fn to_thumbnails(thumbnails: &[rustypipe::model::Thumbnail]) -> Vec<YtdlpThumbnail> {
    thumbnails
        .iter()
        .map(|thumbnail| YtdlpThumbnail {
            url: thumbnail.url.clone(),
            width: Some(thumbnail.width),
            height: Some(thumbnail.height),
        })
        .collect()
}

pub async fn search(
    query: String,
    max_results: Option<u32>,
) -> Result<Vec<YtdlpSearchResult>, String> {
    let limit = max_results.unwrap_or(10) as usize;
    debug!("[ytdlp-mobile] Searching: {} (limit: {})", query, limit);

    let search = client()?
        .query()
        .search::<VideoItem, _>(&query)
        .await
        .map_err(|error| format!("Search failed: {error}"))?;

    let results: Vec<YtdlpSearchResult> = search
        .items
        .items
        .into_iter()
        .take(limit)
        .map(|video| YtdlpSearchResult {
            id: video.id,
            title: video.name,
            duration: video.duration.map(f64::from),
            thumbnail: best_thumbnail_url(&video.thumbnail),
            channel: video.channel.map(|channel| channel.name),
        })
        .collect();

    debug!("[ytdlp-mobile] Found {} results", results.len());
    Ok(results)
}

pub async fn get_stream(video_id: String) -> Result<YtdlpStreamInfo, String> {
    debug!("[ytdlp-mobile] Getting stream for: {}", video_id);

    let player = client()?
        .query()
        .player(&video_id)
        .await
        .map_err(|error| format!("Stream resolution failed: {error}"))?;

    // Mirror yt-dlp's "bestaudio[ext=m4a]/bestaudio[ext=webm]/bestaudio".
    let best = player
        .audio_streams
        .iter()
        .filter(|stream| stream.mime.contains("mp4"))
        .max_by_key(|stream| stream.bitrate)
        .or_else(|| {
            player
                .audio_streams
                .iter()
                .filter(|stream| stream.mime.contains("webm"))
                .max_by_key(|stream| stream.bitrate)
        })
        .or_else(|| player.audio_streams.iter().max_by_key(|stream| stream.bitrate))
        .ok_or_else(|| format!("No audio streams resolved for {video_id}"))?;

    let container = if best.mime.contains("mp4") {
        Some("m4a".to_string())
    } else if best.mime.contains("webm") {
        Some("webm".to_string())
    } else {
        None
    };
    // "audio/mp4; codecs=\"mp4a.40.2\"" -> "mp4a.40.2"
    let codec = best
        .mime
        .split_once("codecs=\"")
        .and_then(|(_, rest)| rest.split('"').next())
        .map(str::to_string);

    Ok(YtdlpStreamInfo {
        stream_url: best.url.clone(),
        duration: Some(f64::from(player.details.duration)),
        title: player.details.name.clone(),
        container,
        codec,
    })
}

pub async fn get_playlist(url: String) -> Result<YtdlpPlaylistInfo, String> {
    debug!("[ytdlp-mobile] Getting playlist: {}", url);

    // Accept both raw playlist ids and full URLs (...?list=<id>&...).
    let playlist_id = url
        .split_once("list=")
        .map(|(_, rest)| rest.split('&').next().unwrap_or(rest).to_string())
        .unwrap_or_else(|| url.clone());

    let playlist = client()?
        .query()
        .playlist(&playlist_id)
        .await
        .map_err(|error| format!("Playlist fetch failed: {error}"))?;

    let entries: Vec<YtdlpPlaylistEntry> = playlist
        .videos
        .items
        .iter()
        .map(|video| YtdlpPlaylistEntry {
            id: video.id.clone(),
            title: video.name.clone(),
            duration: video.duration.map(f64::from),
            thumbnails: to_thumbnails(&video.thumbnail),
            channel: video.channel.as_ref().map(|channel| channel.name.clone()),
        })
        .collect();

    debug!(
        "[ytdlp-mobile] Playlist '{}' has {} entries",
        playlist.name,
        entries.len()
    );

    Ok(YtdlpPlaylistInfo {
        id: playlist.id,
        title: playlist.name,
        entries,
    })
}
