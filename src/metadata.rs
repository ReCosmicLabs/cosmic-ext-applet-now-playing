use std::path::PathBuf;

use mpris::{LoopStatus, PlayerFinder};

use crate::fl;
use crate::player::{
    album_art_dimensions, album_art_path_from_metadata, find_selected_or_active_from_players,
    playback_state_from_player, player_sources_from_players,
};
use crate::window::PlaybackState;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlaybackCapabilities {
    pub seek: bool,
    pub previous: bool,
    pub next: bool,
    pub shuffle: bool,
    pub loop_mode: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NowPlayingData {
    pub text: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub player_bus_name: String,
    pub sources: Vec<(String, String)>,
    pub duration_seconds: Option<u64>,
    pub position_seconds: Option<u64>,
    pub capabilities: PlaybackCapabilities,
    pub shuffle: bool,
    pub volume: Option<u32>,
    pub loop_status: LoopStatus,
    pub state: PlaybackState,
    pub album_art_path: Option<PathBuf>,
    pub album_art_dimensions: Option<(u32, u32)>,
    pub has_active_media: bool,
}

impl NowPlayingData {
    /// Compares the state that matters while the popup is closed. Playback
    /// position is deliberately excluded so an active player does not wake
    /// the applet once a second merely to redraw an invisible seek bar.
    pub fn same_except_position(&self, other: &Self) -> bool {
        self.text == other.text
            && self.title == other.title
            && self.artist == other.artist
            && self.album == other.album
            && self.player_bus_name == other.player_bus_name
            && self.sources == other.sources
            && self.duration_seconds == other.duration_seconds
            && self.capabilities == other.capabilities
            && self.shuffle == other.shuffle
            && self.volume == other.volume
            && self.loop_status == other.loop_status
            && self.state == other.state
            && self.album_art_path == other.album_art_path
            && self.album_art_dimensions == other.album_art_dimensions
            && self.has_active_media == other.has_active_media
    }
}

pub fn now_playing_snapshot(include_position: bool) -> NowPlayingData {
    let finder = PlayerFinder::new();

    if let Ok(finder) = finder {
        if let Ok(players) = finder.find_all() {
            let sources = player_sources_from_players(&players);
            if let Some(player) = find_selected_or_active_from_players(players) {
                return now_playing_from_player_with_sources(&player, sources, include_position);
            }
        }
    }

    NowPlayingData {
        text: fl!("nothing-playing"),
        title: fl!("nothing-playing"),
        artist: String::new(),
        album: String::new(),
        player_bus_name: String::new(),
        sources: Vec::new(),
        duration_seconds: None,
        position_seconds: None,
        shuffle: false,
        volume: None,
        loop_status: LoopStatus::None,
        capabilities: PlaybackCapabilities {
            seek: false,
            previous: false,
            next: false,
            shuffle: false,
            loop_mode: false,
        },
        state: PlaybackState::Stopped,
        album_art_path: None,
        album_art_dimensions: None,
        has_active_media: false,
    }
}

pub fn now_playing_from_player_with_sources(
    player: &mpris::Player,
    sources: Vec<(String, String)>,
    include_position: bool,
) -> NowPlayingData {
    let playback_state = playback_state_from_player(player);

    if let Ok(meta) = player.get_metadata() {
        let title = meta
            .title()
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| fl!("unknown-title"));
        let artist = meta
            .artists()
            .and_then(|a| a.first().copied())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| fl!("unknown-artist"));
        let album_art_path = album_art_path_from_metadata(&meta);
        let album_art_dimensions = album_art_path.as_deref().and_then(album_art_dimensions);

        return NowPlayingData {
            text: format!("{title} • {artist}"),
            title,
            artist,
            album: meta.album_name().unwrap_or_default().to_owned(),
            player_bus_name: player.bus_name().to_owned(),
            sources,
            duration_seconds: meta.length().map(|duration| duration.as_secs()),
            position_seconds: include_position
                .then(|| {
                    player
                        .get_position()
                        .ok()
                        .map(|duration| duration.as_secs())
                })
                .flatten(),
            capabilities: PlaybackCapabilities {
                seek: player.can_seek().unwrap_or(false),
                previous: player.can_go_previous().unwrap_or(false),
                next: player.can_go_next().unwrap_or(false),
                shuffle: player.can_shuffle().unwrap_or(false),
                loop_mode: player.can_loop().unwrap_or(false),
            },
            shuffle: player.get_shuffle().unwrap_or(false),
            volume: player
                .get_volume()
                .ok()
                .map(|v| (v * 100.0).round().clamp(0.0, 100.0) as u32),
            loop_status: player.get_loop_status().unwrap_or(LoopStatus::None),
            state: playback_state,
            album_art_path,
            album_art_dimensions,
            has_active_media: true,
        };
    }

    NowPlayingData {
        text: fl!("nothing-playing"),
        title: fl!("nothing-playing"),
        artist: String::new(),
        album: String::new(),
        player_bus_name: String::new(),
        sources: Vec::new(),
        duration_seconds: None,
        position_seconds: None,
        capabilities: PlaybackCapabilities {
            seek: false,
            previous: false,
            next: false,
            shuffle: false,
            loop_mode: false,
        },
        shuffle: false,
        volume: None,
        loop_status: LoopStatus::None,
        state: playback_state,
        album_art_path: None,
        album_art_dimensions: None,
        has_active_media: false,
    }
}
