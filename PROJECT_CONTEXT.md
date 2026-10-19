# Project Context for Fringe

## Overview
Fringe is a terminal music streamer written in Rust (Ratatui, Rodio, Tokio). It streams Spotify tracks and playlists via YouTube backend (yt-dlp + ffmpeg PCM pipes) with no Spotify Premium required.

## Key Architecture & Modules
- `src/main.rs`: Terminal initialization, keyboard event loop, panic hook with clean raw mode restoration, automatic restore of last loaded playlist on startup.
- `src/app.rs`: Application state, track queue, playlist persistence (`~/.config/fringe/last_playlist.txt`), playback state and AFK screensaver trigger.
- `src/ui.rs`: Ratatui UI layout: Header with status & volume mixer slider, Body (left album cover + right playlist table), Spectrum Visualizer (CAVA / external PTY), Footer, AFK views.
- `src/audio.rs`: Low-latency PCM pipe feeder, disk caching, instant generation killing to prevent audio artifacts or double streams.
- `src/visualizer.rs`: Authentic CAVA physics (logarithmic frequency bands, gravity falloff, Monstercat filter).
- `src/external_vis.rs`: Virtual PTY terminal runner for embedding external CLI tools (`lavat`, `cava`, etc.) directly into visualizer window.
- `src/cover.rs`: Multi-source album art fetcher with Kitty/iTerm2 protocols and Unicode half-block fallback.
- `src/lyrics.rs`: LRCLIB word-by-word synchronized karaoke lyrics client.
- `src/spotify.rs`: Embed scraper for playlist/album/track metadata without requiring API keys.

## Build and Run
- `cargo build --release` -> Binary: `target/release/fringe`
- Config file: `~/.config/fringe/fringe.conf`
- Last playlist cache: `~/.config/fringe/last_playlist.txt`
- Tests / Checks: `cargo check`
