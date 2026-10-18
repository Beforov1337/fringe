use anyhow::{anyhow, Context, Result};
use regex::Regex;
use serde_json::Value;

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct TrackItem {
    pub title: String,
    pub artist: String,
    pub duration_ms: u64,
    pub cover_url: Option<String>,
    pub spotify_uri: Option<String>,
}

impl TrackItem {
    pub fn duration_formatted(&self) -> String {
        let total_secs = self.duration_ms / 1000;
        let mins = total_secs / 60;
        let secs = total_secs % 60;
        format!("{:02}:{:02}", mins, secs)
    }

    pub fn search_query(&self) -> String {
        format!("{} {} Topic", self.artist, self.title)
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct PlaylistMeta {
    pub title: String,
    pub description: String,
    pub cover_url: Option<String>,
    pub tracks: Vec<TrackItem>,
}

#[derive(Debug, Clone)]
pub struct PresetPlaylist {
    pub name: &'static str,
    pub desc: &'static str,
    pub url: &'static str,
}

pub const PRESETS: &[PresetPlaylist] = &[
    PresetPlaylist {
        name: "Today's Top Hits",
        desc: "Spotify's biggest global pop hits",
        url: "https://open.spotify.com/playlist/37i9dQZF1DXcBWIGoYBM5M",
    },
    PresetPlaylist {
        name: "Lofi Beats",
        desc: "Beats to relax, study, and code to",
        url: "https://open.spotify.com/playlist/37i9dQZF1DXdLEN7aqioXM",
    },
    PresetPlaylist {
        name: "Synthwave / Cyberpunk",
        desc: "Retro-futuristic synth and darkwave",
        url: "https://open.spotify.com/playlist/37i9dQZF1DXdLEN7aqioXM",
    },
    PresetPlaylist {
        name: "Rock Classics",
        desc: "Timeless rock and guitar legends",
        url: "https://open.spotify.com/playlist/37i9dQZF1DWXRqgorJj26U",
    },
    PresetPlaylist {
        name: "Chillhop Essentials",
        desc: "Smooth jazzy chill hip hop",
        url: "https://open.spotify.com/playlist/37i9dQZF1DX3qCx52y9omv",
    },
];

pub struct SpotifyClient {
    client: reqwest::Client,
}

impl SpotifyClient {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .user_agent("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36")
            .build()
            .unwrap_or_default();
        Self { client }
    }

    /// Parse link or URI into (entity_type, id)
    /// Supports:
    /// - open.spotify.com/playlist/{id}
    /// - open.spotify.com/album/{id}
    /// - open.spotify.com/track/{id}
    /// - spotify:playlist:{id}
    /// - spotify:album:{id}
    /// - spotify:track:{id}
    pub fn parse_spotify_link(input: &str) -> Option<(String, String)> {
        let input = input.trim();

        // Check URI format: spotify:playlist:xxxx
        if let Some(stripped) = input.strip_prefix("spotify:") {
            let parts: Vec<&str> = stripped.split(':').collect();
            if parts.len() >= 2 {
                return Some((parts[0].to_string(), parts[1].to_string()));
            }
        }

        // Check Web URL format: https://open.spotify.com/playlist/xxxx?si=yyyy
        let re = Regex::new(r"spotify\.com/(playlist|album|track)/([a-zA-Z0-9]+)").ok()?;
        if let Some(caps) = re.captures(input) {
            let entity_type = caps.get(1)?.as_str().to_string();
            let id = caps.get(2)?.as_str().to_string();
            return Some((entity_type, id));
        }

        // Check if raw alphanumeric ID is provided (default to playlist)
        if !input.is_empty() && input.chars().all(|c| c.is_ascii_alphanumeric()) {
            return Some(("playlist".to_string(), input.to_string()));
        }

        None
    }

    pub async fn fetch_entity(&self, input: &str) -> Result<PlaylistMeta> {
        let (entity_type, id) = Self::parse_spotify_link(input)
            .ok_or_else(|| anyhow!("Invalid Spotify URL or URI: '{}'", input))?;

        let embed_url = format!("https://open.spotify.com/embed/{}/{}", entity_type, id);

        let response = self
            .client
            .get(&embed_url)
            .send()
            .await
            .context("Failed to send HTTP request to Spotify embed")?;

        if !response.status().is_success() {
            return Err(anyhow!(
                "Spotify embed returned HTTP status {}",
                response.status()
            ));
        }

        let html = response
            .text()
            .await
            .context("Failed to read Spotify embed response body")?;

        let next_data_regex =
            Regex::new(r#"<script id="__NEXT_DATA__" type="application/json">(.*?)</script>"#)
                .context("Failed to compile regex")?;

        let captures = next_data_regex
            .captures(&html)
            .ok_or_else(|| anyhow!("Could not find __NEXT_DATA__ in Spotify embed HTML"))?;

        let json_str = captures
            .get(1)
            .ok_or_else(|| anyhow!("Empty JSON block in embed HTML"))?
            .as_str();

        let val: Value = serde_json::from_str(json_str)
            .context("Failed to parse Spotify embed JSON data")?;

        let entity = &val["props"]["pageProps"]["state"]["data"]["entity"];
        if entity.is_null() {
            return Err(anyhow!("Missing 'entity' field in Spotify response"));
        }

        match entity_type.as_str() {
            "playlist" | "album" => {
                let title = entity["name"]
                    .as_str()
                    .or_else(|| entity["title"].as_str())
                    .unwrap_or("Spotify Playlist")
                    .to_string();

                let desc = entity["subtitle"]
                    .as_str()
                    .unwrap_or_else(|| {
                        if entity_type == "album" {
                            "Album"
                        } else {
                            "Spotify Playlist"
                        }
                    })
                    .to_string();

                let cover_url = entity["coverArt"]["sources"]
                    .as_array()
                    .and_then(|arr| arr.first())
                    .and_then(|s| s["url"].as_str())
                    .map(|s| s.to_string());

                let mut tracks = Vec::new();

                if let Some(track_list) = entity["trackList"].as_array() {
                    for t in track_list {
                        let t_title = t["title"].as_str().unwrap_or("Unknown Track").to_string();
                        let t_artist = t["subtitle"].as_str().unwrap_or("Unknown Artist").to_string();
                        let t_dur = t["duration"].as_u64().unwrap_or(0);
                        let uri = t["uri"].as_str().map(|s| s.to_string());

                        tracks.push(TrackItem {
                            title: t_title,
                            artist: t_artist,
                            duration_ms: t_dur,
                            cover_url: cover_url.clone(),
                            spotify_uri: uri,
                        });
                    }
                }

                Ok(PlaylistMeta {
                    title,
                    description: desc,
                    cover_url,
                    tracks,
                })
            }
            "track" => {
                let title = entity["title"]
                    .as_str()
                    .unwrap_or("Spotify Track")
                    .to_string();

                let artist = entity["artists"]
                    .as_array()
                    .and_then(|arr| arr.first())
                    .and_then(|a| a["name"].as_str())
                    .unwrap_or("Unknown Artist")
                    .to_string();

                let duration_ms = entity["duration"].as_u64().unwrap_or(0);

                let cover_url = entity["visualIdentity"]["image"]
                    .as_array()
                    .and_then(|arr| arr.first())
                    .and_then(|img| img["url"].as_str())
                    .map(|s| s.to_string());

                let track = TrackItem {
                    title: title.clone(),
                    artist: artist.clone(),
                    duration_ms,
                    cover_url: cover_url.clone(),
                    spotify_uri: entity["uri"].as_str().map(|s| s.to_string()),
                };

                Ok(PlaylistMeta {
                    title,
                    description: format!("Single by {}", artist),
                    cover_url,
                    tracks: vec![track],
                })
            }
            _ => Err(anyhow!("Unsupported Spotify entity type: {}", entity_type)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_spotify_links() {
        assert_eq!(
            SpotifyClient::parse_spotify_link("https://open.spotify.com/playlist/37i9dQZF1DXcBWIGoYBM5M?si=123"),
            Some(("playlist".to_string(), "37i9dQZF1DXcBWIGoYBM5M".to_string()))
        );
        assert_eq!(
            SpotifyClient::parse_spotify_link("https://open.spotify.com/album/4yP0hdKOJeuuk8owLQj79m"),
            Some(("album".to_string(), "4yP0hdKOJeuuk8owLQj79m".to_string()))
        );
        assert_eq!(
            SpotifyClient::parse_spotify_link("spotify:track:3XmwAZpIMNpTJy8faw3ajD"),
            Some(("track".to_string(), "3XmwAZpIMNpTJy8faw3ajD".to_string()))
        );
    }
}
