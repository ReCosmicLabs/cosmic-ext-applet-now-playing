use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

use mpris::{LoopStatus, PlaybackStatus, PlayerFinder};
use url::Url;

use crate::window::PlaybackState;

type ArtworkDimensions = Option<(u32, u32)>;
type ArtworkDimensionsCache = HashMap<PathBuf, ArtworkDimensions>;

static SELECTED_PLAYER: OnceLock<Mutex<Option<String>>> = OnceLock::new();
static ART_DOWNLOADS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
static ART_FAILURES: OnceLock<Mutex<HashMap<String, Instant>>> = OnceLock::new();
static ART_DIMENSIONS: OnceLock<Mutex<ArtworkDimensionsCache>> = OnceLock::new();
static LAST_CACHE_PRUNE: OnceLock<Mutex<Option<Instant>>> = OnceLock::new();

const MAX_ART_DOWNLOADS: usize = 2;
const MAX_ART_BYTES: u64 = 10 * 1024 * 1024;
const MAX_ART_CACHE_BYTES: u64 = 100 * 1024 * 1024;
const MAX_ART_CACHE_AGE: Duration = Duration::from_secs(30 * 24 * 60 * 60);
const CACHE_PRUNE_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

fn selected_player_slot() -> &'static Mutex<Option<String>> {
    SELECTED_PLAYER.get_or_init(|| Mutex::new(None))
}

pub fn select_player(bus_name: String) {
    if let Ok(mut selected) = selected_player_slot().lock() {
        *selected = Some(bus_name);
    }
}

/// Select the explicitly chosen source when it still exists; otherwise follow
/// MPRIS' active-player preference (playing, paused, metadata, first player).
/// Taking the already enumerated player list lets a poll build its source list
/// and chosen snapshot from a single DBus enumeration.
pub fn find_selected_or_active_from_players(players: Vec<mpris::Player>) -> Option<mpris::Player> {
    let selected = selected_player_slot()
        .lock()
        .ok()
        .and_then(|selected| selected.clone());
    if let Some(bus_name) = selected {
        if let Some(player) = players
            .iter()
            .position(|player| player.bus_name() == bus_name)
        {
            return players.into_iter().nth(player);
        }
        // A selected player which has exited should not trap the applet on an
        // empty source forever.
        if let Ok(mut selected) = selected_player_slot().lock() {
            *selected = None;
        }
    }
    // A dedicated music player wins over whatever a browser tab happens to expose.
    let mut players = players;
    players.sort_by_key(player_rank);
    if let Some(player) = players.iter().position(|player| {
        player_rank(player) == 0
            && player
                .get_metadata()
                .map(|metadata| !metadata.is_empty())
                .unwrap_or(false)
    }) {
        return players.into_iter().nth(player);
    }
    let mut first_paused = None;
    let mut first_with_track = None;
    let mut first_found = None;
    for player in players {
        match player.get_playback_status() {
            Ok(PlaybackStatus::Playing) => return Some(player),
            Ok(PlaybackStatus::Paused) if first_paused.is_none() => first_paused = Some(player),
            _ if first_with_track.is_none()
                && player
                    .get_metadata()
                    .map(|metadata| !metadata.is_empty())
                    .unwrap_or(false) =>
            {
                first_with_track = Some(player)
            }
            _ if first_found.is_none() => first_found = Some(player),
            _ => {}
        }
    }
    first_paused.or(first_with_track).or(first_found)
}

const PREFERRED_PLAYERS: &[&str] = &["spotify"];
const DEMOTED_PLAYERS: &[&str] = &["chromium", "chrome", "firefox", "brave", "vivaldi"];

/// 0 = preferred music player, 1 = anything else, 2 = a browser.
fn player_rank(player: &mpris::Player) -> u8 {
    let bus = player.bus_name().to_ascii_lowercase();
    if PREFERRED_PLAYERS.iter().any(|p| bus.contains(p)) {
        0
    } else if DEMOTED_PLAYERS.iter().any(|p| bus.contains(p)) {
        2
    } else {
        1
    }
}

pub fn player_icon_name(bus_name: &str) -> &'static str {
    let bus = bus_name.to_ascii_lowercase();
    if bus.contains("spotify") {
        "spotify-launcher"
    } else if bus.contains("chromium") || bus.contains("chrome") {
        "google-chrome"
    } else if bus.contains("firefox") {
        "firefox"
    } else {
        "audio-x-generic-symbolic"
    }
}

pub fn playback_state_from_player(player: &mpris::Player) -> PlaybackState {
    match player.get_playback_status() {
        Ok(PlaybackStatus::Playing) => PlaybackState::Playing,
        Ok(PlaybackStatus::Paused) => PlaybackState::Paused,
        Ok(PlaybackStatus::Stopped) => PlaybackState::Stopped,
        Err(_) => PlaybackState::Unknown,
    }
}

pub fn album_art_path_from_metadata(meta: &mpris::Metadata) -> Option<PathBuf> {
    let art_url = meta.art_url()?;
    file_url_to_path(art_url).or_else(|| download_remote_album_art(art_url))
}

/// Read dimensions once per artwork path so the panel can reserve the
/// thumbnail's natural width without decoding it during every redraw.
pub fn album_art_dimensions(path: &Path) -> ArtworkDimensions {
    let read_dimensions = || {
        imagesize::size(path).ok().and_then(|size| {
            u32::try_from(size.width)
                .ok()
                .zip(u32::try_from(size.height).ok())
        })
    };

    let dimensions = ART_DIMENSIONS.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(mut dimensions) = dimensions.lock() {
        return *dimensions
            .entry(path.to_owned())
            .or_insert_with(read_dimensions);
    }
    read_dimensions()
}

/// MPRIS players are allowed to publish either a local `file://` URL or a
/// remote HTTP(S) URL for cover art. Browsers and Spotify commonly use the
/// latter, so cache it locally before handing it to COSMIC's image widget.
fn download_remote_album_art(url: &str) -> Option<PathBuf> {
    if !is_downloadable_art_url(url) {
        return None;
    }

    let cache_dir = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))?
        .join("cosmic-ext-applet-now-playing");
    fs::create_dir_all(&cache_dir).ok()?;
    prune_album_art_cache(&cache_dir);

    let path = cache_dir.join(format!("{}.{}", stable_url_hash(url), image_extension(url)));
    if path.is_file() {
        return Some(path);
    }

    let failures = ART_FAILURES.get_or_init(|| Mutex::new(HashMap::new()));
    if failures
        .lock()
        .ok()
        .and_then(|failures| failures.get(url).copied())
        .is_some_and(|last_failure| last_failure.elapsed() < Duration::from_secs(30))
    {
        return None;
    }

    let downloads = ART_DOWNLOADS.get_or_init(|| Mutex::new(HashSet::new()));
    let should_start = downloads.lock().ok().is_some_and(|mut downloads| {
        downloads.len() < MAX_ART_DOWNLOADS && downloads.insert(url.to_owned())
    });
    if !should_start {
        return None;
    }

    let url = url.to_owned();
    std::thread::spawn(move || {
        let temporary = path.with_extension(format!("{}.part", image_extension(&url)));
        let result = (|| -> io::Result<()> {
            let response = ureq::get(&url)
                .timeout(Duration::from_secs(8))
                .call()
                .map_err(|error| io::Error::other(error.to_string()))?;
            if response
                .header("Content-Length")
                .and_then(|length| length.parse::<u64>().ok())
                .is_some_and(|length| length > MAX_ART_BYTES)
            {
                return Err(io::Error::other("album art exceeds the download limit"));
            }
            let mut reader = response.into_reader();
            let mut file = fs::File::create(&temporary)?;
            let copied = io::copy(&mut reader.by_ref().take(MAX_ART_BYTES + 1), &mut file)?;
            if copied > MAX_ART_BYTES {
                return Err(io::Error::other("album art exceeds the download limit"));
            }
            fs::rename(&temporary, &path)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
            if let Ok(mut failures) = ART_FAILURES
                .get_or_init(|| Mutex::new(HashMap::new()))
                .lock()
            {
                failures.insert(url.clone(), Instant::now());
            }
        }
        if let Ok(mut downloads) = ART_DOWNLOADS
            .get_or_init(|| Mutex::new(HashSet::new()))
            .lock()
        {
            downloads.remove(&url);
        }
    });
    None
}

fn is_downloadable_art_url(url: &str) -> bool {
    url.starts_with("https://")
}

fn prune_album_art_cache(cache_dir: &Path) {
    let now = Instant::now();
    let should_prune = LAST_CACHE_PRUNE
        .get_or_init(|| Mutex::new(None))
        .lock()
        .ok()
        .is_some_and(|mut last_prune| {
            if last_prune.is_some_and(|last| last.elapsed() < CACHE_PRUNE_INTERVAL) {
                false
            } else {
                *last_prune = Some(now);
                true
            }
        });
    if !should_prune {
        return;
    }

    prune_album_art_cache_with_limits(
        cache_dir,
        SystemTime::now(),
        MAX_ART_CACHE_AGE,
        MAX_ART_CACHE_BYTES,
    );
}

fn prune_album_art_cache_with_limits(
    cache_dir: &Path,
    now: SystemTime,
    max_age: Duration,
    max_bytes: u64,
) {
    let Ok(entries) = fs::read_dir(cache_dir) else {
        return;
    };
    let mut files = Vec::new();
    let mut total_size = 0_u64;
    for entry in entries.flatten() {
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        if entry
            .path()
            .extension()
            .is_some_and(|extension| extension == "part")
        {
            continue;
        }
        let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        if now.duration_since(modified).unwrap_or_default() > max_age {
            let _ = fs::remove_file(entry.path());
            continue;
        }
        total_size = total_size.saturating_add(metadata.len());
        files.push((modified, metadata.len(), entry.path()));
    }

    files.sort_by_key(|(modified, _, _)| *modified);
    for (_, size, path) in files {
        if total_size <= max_bytes {
            break;
        }
        if fs::remove_file(path).is_ok() {
            total_size = total_size.saturating_sub(size);
        }
    }
}

fn stable_url_hash(url: &str) -> u64 {
    // FNV-1a gives the cache a stable, filesystem-safe name without adding a
    // hashing dependency or storing the remote URL in a filename.
    url.as_bytes()
        .iter()
        .fold(0xcbf2_9ce4_8422_2325u64, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
        })
}

fn image_extension(url: &str) -> &'static str {
    let path = url.split('?').next().unwrap_or(url).to_ascii_lowercase();
    if path.ends_with(".png") {
        "png"
    } else if path.ends_with(".webp") {
        "webp"
    } else if path.ends_with(".jpeg") {
        "jpeg"
    } else {
        "jpg"
    }
}

fn file_url_to_path(url: &str) -> Option<PathBuf> {
    if !has_valid_percent_encoding(url) {
        return None;
    }
    Url::parse(url).ok()?.to_file_path().ok()
}

fn has_valid_percent_encoding(value: &str) -> bool {
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len()
                || !bytes[index + 1].is_ascii_hexdigit()
                || !bytes[index + 2].is_ascii_hexdigit()
            {
                return false;
            }
            index += 3;
        } else {
            index += 1;
        }
    }
    true
}

pub fn player_sources_from_players(players: &[mpris::Player]) -> Vec<(String, String)> {
    players
        .iter()
        .map(|player| (player.bus_name().to_owned(), player.identity().to_owned()))
        .collect()
}

pub fn with_player<F, T>(bus_name: &str, f: F) -> Option<T>
where
    F: FnOnce(&mpris::Player) -> T,
{
    if let Ok(finder) = PlayerFinder::new() {
        if let Ok(players) = finder.find_all() {
            if let Some(player) = players
                .into_iter()
                .find(|player| player.bus_name() == bus_name)
            {
                return Some(f(&player));
            }
        }
    }
    None
}

pub fn toggle_shuffle(player: &mpris::Player) -> Option<bool> {
    let current = player.checked_get_shuffle().ok().flatten()?;
    let next = !current;
    player
        .checked_set_shuffle(next)
        .ok()
        .filter(|changed| *changed)?;
    // Cider currently advertises a writable property but silently retains its
    // old value. Confirm the write before reflecting it in the applet.
    player.get_shuffle().ok().filter(|actual| *actual == next)
}

pub fn cycle_loop_status(player: &mpris::Player) -> Option<LoopStatus> {
    let current = player.checked_get_loop_status().ok().flatten()?;
    let next = match current {
        LoopStatus::None => LoopStatus::Playlist,
        LoopStatus::Playlist => LoopStatus::Track,
        LoopStatus::Track => LoopStatus::None,
    };
    player
        .checked_set_loop_status(next)
        .ok()
        .filter(|changed| *changed)?;
    player
        .get_loop_status()
        .ok()
        .filter(|actual| *actual == next)
}

#[cfg(test)]
mod tests {
    use super::{
        album_art_dimensions, file_url_to_path, image_extension, is_downloadable_art_url,
        prune_album_art_cache_with_limits, MAX_ART_CACHE_AGE,
    };
    use std::fs;
    use std::path::PathBuf;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    fn temporary_directory(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "cosmic-now-playing-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn parses_file_url_paths() {
        assert_eq!(
            file_url_to_path("file:///home/user/Music/Album%20Art.png"),
            Some(PathBuf::from("/home/user/Music/Album Art.png"))
        );
    }

    #[test]
    fn handles_localhost_and_unicode_file_urls() {
        assert_eq!(
            file_url_to_path("file://localhost/home/user/M%C3%BAsica.png"),
            Some(PathBuf::from("/home/user/Música.png"))
        );
        assert_eq!(
            file_url_to_path("file://example.com/home/user/cover.png"),
            None
        );
    }

    #[test]
    fn ignores_non_file_urls() {
        assert_eq!(file_url_to_path("https://example.com/cover.png"), None);
    }

    #[test]
    fn rejects_malformed_file_urls() {
        assert_eq!(file_url_to_path("file:///Music/cover%ZZ.png"), None);
    }

    #[test]
    fn chooses_a_safe_cache_extension() {
        assert_eq!(
            image_extension("https://example.com/cover.webp?size=640"),
            "webp"
        );
        assert_eq!(image_extension("https://example.com/cover"), "jpg");
    }

    #[test]
    fn downloads_only_secure_remote_artwork() {
        assert!(is_downloadable_art_url("https://example.com/cover.jpg"));
        assert!(!is_downloadable_art_url("http://example.com/cover.jpg"));
        assert!(!is_downloadable_art_url("file:///tmp/cover.jpg"));
    }

    #[test]
    fn caches_album_art_dimensions() {
        let directory = temporary_directory("dimensions");
        let image = directory.join("cover.png");
        fs::write(
            &image,
            [
                0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13, b'I', b'H', b'D',
                b'R', 0, 0, 0, 2, 0, 0, 0, 3,
            ],
        )
        .unwrap();

        assert_eq!(album_art_dimensions(&image), Some((2, 3)));
        fs::remove_file(&image).unwrap();
        assert_eq!(album_art_dimensions(&image), Some((2, 3)));
        assert_eq!(album_art_dimensions(&directory.join("missing.png")), None);

        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn prunes_expired_and_oversized_album_art_cache() {
        let directory = temporary_directory("prune");
        let expired = directory.join("expired.jpg");
        fs::write(&expired, b"old").unwrap();
        prune_album_art_cache_with_limits(
            &directory,
            SystemTime::now() + MAX_ART_CACHE_AGE + Duration::from_secs(1),
            MAX_ART_CACHE_AGE,
            100,
        );
        assert!(!expired.exists());

        for name in ["one.jpg", "two.jpg", "three.jpg"] {
            fs::write(directory.join(name), [0_u8; 8]).unwrap();
        }
        prune_album_art_cache_with_limits(
            &directory,
            SystemTime::now(),
            Duration::from_secs(365 * 24 * 60 * 60),
            10,
        );
        let remaining_bytes = fs::read_dir(&directory)
            .unwrap()
            .flatten()
            .map(|entry| entry.metadata().unwrap().len())
            .sum::<u64>();
        assert!(remaining_bytes <= 10);

        fs::remove_dir_all(directory).unwrap();
    }
}
