use anyhow::Result;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::Instant;
use tokio::runtime::Handle;

use crate::audio::AudioPlayer;
use crate::config::Config;
use crate::cover::CoverArtRenderer;
use crate::lyrics::{LyricsClient, LyricsData};
use crate::spotify::{PlaylistMeta, SpotifyClient, TrackItem, PRESETS};
use crate::visualizer::CavaVisualizer;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AfkMode {
    Lyrics,
    Spectrum,
}

pub enum AppEvent {
    LyricsLoaded(LyricsData),
    LyricsLoadFailed,
    PlaylistLoaded(PlaylistMeta),
    PlaylistLoadFailed(String),
    PlaybackStarted,
}

pub struct App {
    pub config: Config,

    pub playlist: Option<PlaylistMeta>,
    pub current_track_idx: Option<usize>,
    pub selected_idx: usize,
    pub list_scroll: usize,

    pub is_searching: bool,
    pub search_query: String,

    pub is_url_modal_open: bool,
    pub url_input: String,
    pub active_preset_idx: usize,

    pub shuffle: bool,
    pub repeat: bool,
    pub status_message: String,
    pub is_loading_track: bool,

    // AFK / Screensaver
    pub is_afk: bool,
    pub afk_mode: AfkMode,
    pub last_activity: Instant,

    // Lyrics
    pub lyrics: Option<LyricsData>,
    pub is_loading_lyrics: bool,
    pub lyrics_client: Arc<LyricsClient>,

    // Playback timing
    pub track_started_at: Option<Instant>,
    pub paused_at: Option<Instant>,
    pub total_paused_duration: std::time::Duration,

    pub audio_player: Arc<AudioPlayer>,
    pub cover_renderer: CoverArtRenderer,
    pub visualizer: CavaVisualizer,
    pub external_vis: Option<crate::external_vis::ExternalVisualizer>,

    pub spotify_client: Arc<SpotifyClient>,
    pub event_tx: Sender<AppEvent>,
    pub event_rx: Receiver<AppEvent>,
    pub tokio_handle: Handle,
}

impl App {
    pub fn new(tokio_handle: Handle) -> Result<Self> {
        let config = Config::load_or_create_default();
        let audio_player = Arc::new(AudioPlayer::new()?);
        let cover_renderer = CoverArtRenderer::new();
        let visualizer = CavaVisualizer::new();
        let external_vis = if !config.visualizer_cmd.trim().is_empty() {
            Some(crate::external_vis::ExternalVisualizer::new(config.visualizer_cmd.clone()))
        } else {
            None
        };
        let spotify_client = Arc::new(SpotifyClient::new());
        let lyrics_client = Arc::new(LyricsClient::new());
        let (event_tx, event_rx) = channel();

        Ok(Self {
            config,
            playlist: None,
            current_track_idx: None,
            selected_idx: 0,
            list_scroll: 0,

            is_searching: false,
            search_query: String::new(),

            is_url_modal_open: false,
            url_input: String::new(),
            active_preset_idx: 0,

            shuffle: false,
            repeat: false,
            status_message: "Press 'L' to load a Spotify playlist or select a preset".to_string(),
            is_loading_track: false,

            is_afk: false,
            afk_mode: AfkMode::Lyrics,
            last_activity: Instant::now(),

            lyrics: None,
            is_loading_lyrics: false,
            lyrics_client,

            track_started_at: None,
            paused_at: None,
            total_paused_duration: std::time::Duration::ZERO,

            audio_player,
            cover_renderer,
            visualizer,
            external_vis,

            spotify_client,
            event_tx,
            event_rx,
            tokio_handle,
        })
    }

    pub fn filtered_indices(&self) -> Vec<usize> {
        let Some(ref pl) = self.playlist else {
            return Vec::new();
        };

        if self.search_query.trim().is_empty() {
            (0..pl.tracks.len()).collect()
        } else {
            let q = self.search_query.to_lowercase();
            pl.tracks
                .iter()
                .enumerate()
                .filter(|(_, t)| {
                    t.title.to_lowercase().contains(&q) || t.artist.to_lowercase().contains(&q)
                })
                .map(|(i, _)| i)
                .collect()
        }
    }

    pub fn current_track(&self) -> Option<&TrackItem> {
        let pl = self.playlist.as_ref()?;
        let idx = self.current_track_idx?;
        pl.tracks.get(idx)
    }

    pub fn load_spotify_url(&mut self, url: &str) {
        let url = url.trim().to_string();
        if url.is_empty() {
            return;
        }

        self.status_message = format!("Fetching Spotify metadata for '{}'...", url);
        self.is_url_modal_open = false;
        self.last_activity = Instant::now();

        let client = Arc::clone(&self.spotify_client);
        let tx = self.event_tx.clone();

        self.tokio_handle.spawn(async move {
            match client.fetch_entity(&url).await {
                Ok(meta) => {
                    let _ = tx.send(AppEvent::PlaylistLoaded(meta));
                }
                Err(err) => {
                    let _ = tx.send(AppEvent::PlaylistLoadFailed(err.to_string()));
                }
            }
        });
    }

    pub fn play_track_at(&mut self, track_idx: usize) {
        let Some(ref pl) = self.playlist else {
            return;
        };

        if track_idx >= pl.tracks.len() {
            return;
        }

        let track = pl.tracks[track_idx].clone();
        self.current_track_idx = Some(track_idx);
        self.is_loading_track = true;
        self.status_message = format!("Streaming: {} - {}", track.artist, track.title);
        self.last_activity = Instant::now();

        // 1. Multi-source smart cover art loader
        self.cover_renderer.load_cover_smart(
            track.cover_url.clone(),
            track.spotify_uri.clone(),
            track.search_query(),
            &self.tokio_handle,
        );

        // 2. Fetch lyrics in background
        self.lyrics = None;
        self.is_loading_lyrics = true;
        let l_client = Arc::clone(&self.lyrics_client);
        let l_tx = self.event_tx.clone();
        let artist_clone = track.artist.clone();
        let title_clone = track.title.clone();
        let dur_secs = (track.duration_ms as f64) / 1000.0;

        self.tokio_handle.spawn(async move {
            match l_client.fetch_lyrics(&artist_clone, &title_clone, Some(dur_secs)).await {
                Ok(data) => {
                    let _ = l_tx.send(AppEvent::LyricsLoaded(data));
                }
                Err(_) => {
                    let _ = l_tx.send(AppEvent::LyricsLoadFailed);
                }
            }
        });

        // 3. Pre-buffer the NEXT track into cache so there is 0 delay when this one ends!
        if track_idx + 1 < pl.tracks.len() {
            let next_query = pl.tracks[track_idx + 1].search_query();
            AudioPlayer::prebuffer_track(next_query);
        }

        // 4. Play track: checks local disk cache first (0ms latency), otherwise streams
        let dur_secs = (track.duration_ms as f64) / 1000.0;
        let player = Arc::clone(&self.audio_player);
        let query = track.search_query();

        self.track_started_at = None; // Wait for physical audio start!
        self.paused_at = None;
        self.total_paused_duration = std::time::Duration::ZERO;

        let tx = self.event_tx.clone();
        self.tokio_handle.spawn_blocking(move || {
            let _ = player.play_search_query(query, dur_secs, move || {
                let _ = tx.send(AppEvent::PlaybackStarted);
            });
        });

        self.is_loading_track = false;
    }

    pub fn playback_elapsed(&self) -> f64 {
        let is_playing = self.audio_player.is_playing.load(std::sync::atomic::Ordering::Relaxed);
        if !is_playing {
            return 0.0;
        }

        if let Some(start) = self.track_started_at {
            let now = Instant::now();
            let mut paused_dur = self.total_paused_duration;
            if let Some(p_at) = self.paused_at {
                paused_dur += now.saturating_duration_since(p_at);
            }
            let total_dur = now.saturating_duration_since(start);
            total_dur.saturating_sub(paused_dur).as_secs_f64()
        } else {
            0.0
        }
    }

    pub fn next_track(&mut self) {
        let Some(ref pl) = self.playlist else { return };
        if pl.tracks.is_empty() { return };

        let next_idx = if self.shuffle {
            fastrand::usize(0..pl.tracks.len())
        } else {
            match self.current_track_idx {
                Some(cur) => {
                    if cur + 1 < pl.tracks.len() {
                        cur + 1
                    } else if self.repeat {
                        0
                    } else {
                        return;
                    }
                }
                None => 0,
            }
        };

        self.selected_idx = next_idx;
        self.play_track_at(next_idx);
    }

    pub fn prev_track(&mut self) {
        let Some(ref pl) = self.playlist else { return };
        if pl.tracks.is_empty() { return };

        let prev_idx = match self.current_track_idx {
            Some(cur) => {
                if cur > 0 {
                    cur - 1
                } else if self.repeat {
                    pl.tracks.len().saturating_sub(1)
                } else {
                    0
                }
            }
            None => 0,
        };

        self.selected_idx = prev_idx;
        self.play_track_at(prev_idx);
    }

    pub fn toggle_play_pause(&mut self) {
        if self.current_track_idx.is_none() {
            if let Some(ref pl) = self.playlist {
                if !pl.tracks.is_empty() {
                    self.play_track_at(self.selected_idx);
                }
            }
            return;
        }

        self.audio_player.toggle_pause();
        let is_paused = self.audio_player.is_paused();
        if is_paused {
            self.paused_at = Some(Instant::now());
            self.status_message = "Paused".to_string();
        } else {
            if let Some(p_at) = self.paused_at.take() {
                self.total_paused_duration += Instant::now().saturating_duration_since(p_at);
            }
            self.status_message = "Playing".to_string();
        }
    }

    pub fn process_events(&mut self) {
        while let Ok(event) = self.event_rx.try_recv() {
            match event {
                AppEvent::PlaylistLoaded(meta) => {
                    let track_count = meta.tracks.len();
                    self.status_message = format!("Loaded '{}' ({} tracks)", meta.title, track_count);
                    if let Some(ref cover) = meta.cover_url {
                        self.cover_renderer.load_cover_smart(
                            Some(cover.clone()),
                            None,
                            meta.title.clone(),
                            &self.tokio_handle,
                        );
                    }
                    self.playlist = Some(meta);
                    self.selected_idx = 0;
                    self.list_scroll = 0;

                    if track_count > 0 {
                        self.play_track_at(0);
                    }
                }
                AppEvent::PlaylistLoadFailed(err) => {
                    self.status_message = format!("Failed to load Spotify link: {}", err);
                }
                AppEvent::LyricsLoaded(lyrics) => {
                    self.is_loading_lyrics = false;
                    self.lyrics = Some(lyrics);
                }
                AppEvent::LyricsLoadFailed => {
                    self.is_loading_lyrics = false;
                }
                AppEvent::PlaybackStarted => {
                    self.track_started_at = Some(Instant::now());
                    self.paused_at = None;
                    self.total_paused_duration = std::time::Duration::ZERO;
                }
            }
        }

        // Auto-advance track when current one reaches total duration (Prebuffered -> 0s gap!)
        let is_playing = self.audio_player.is_playing.load(std::sync::atomic::Ordering::Relaxed);
        let elapsed = self.playback_elapsed();
        let total = *self.audio_player.total_secs.lock().unwrap_or_else(|e| e.into_inner());

        if is_playing && total > 5.0 && elapsed >= total - 0.4 {
            self.next_track();
        }

        // Inactivity AFK trigger: 10 seconds of no key presses while playing song
        if !self.is_afk && is_playing && self.last_activity.elapsed() >= std::time::Duration::from_secs(10) {
            self.is_afk = true;
        }
    }

    pub fn select_next(&mut self) {
        let indices = self.filtered_indices();
        if indices.is_empty() { return };

        let pos = indices.iter().position(|&i| i == self.selected_idx).unwrap_or(0);
        if pos + 1 < indices.len() {
            self.selected_idx = indices[pos + 1];
        }
    }

    pub fn select_prev(&mut self) {
        let indices = self.filtered_indices();
        if indices.is_empty() { return };

        let pos = indices.iter().position(|&i| i == self.selected_idx).unwrap_or(0);
        if pos > 0 {
            self.selected_idx = indices[pos - 1];
        }
    }

    pub fn select_preset_next(&mut self) {
        if self.active_preset_idx + 1 < PRESETS.len() {
            self.active_preset_idx += 1;
            self.url_input = PRESETS[self.active_preset_idx].url.to_string();
        }
    }

    pub fn select_preset_prev(&mut self) {
        if self.active_preset_idx > 0 {
            self.active_preset_idx -= 1;
            self.url_input = PRESETS[self.active_preset_idx].url.to_string();
        }
    }
}
