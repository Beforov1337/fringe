use anyhow::{anyhow, Result};
use regex::Regex;
use serde_json::Value;

#[derive(Debug, Clone)]
pub struct SyncedLine {
    pub time_secs: f64,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ActiveLyricStatus {
    Intro,
    Singing { line_idx: usize, word_idx: usize },
    InstrumentalBreak { prev_line_idx: usize, next_line_idx: usize },
    AllFinished,
}

#[derive(Debug, Clone, Default)]
pub struct LyricsData {
    pub lines: Vec<SyncedLine>,
    pub plain_lines: Vec<String>,
    pub has_synced: bool,
}

impl LyricsData {
    /// Accurate real-time status: finds current line and calculates natural word timing based on syllables/chars
    pub fn get_active_status(&self, elapsed_secs: f64, total_duration: f64) -> ActiveLyricStatus {
        if self.has_synced && !self.lines.is_empty() {
            // 1. Intro check: before the first sung line (with small 0.3s lead-in tolerance)
            if elapsed_secs < (self.lines[0].time_secs - 0.3).max(0.0) {
                return ActiveLyricStatus::Intro;
            }

            // 2. Find current line index by timestamp
            let mut cur = 0;
            for (i, line) in self.lines.iter().enumerate() {
                if line.time_secs <= elapsed_secs {
                    cur = i;
                } else {
                    break;
                }
            }

            let start_t = self.lines[cur].time_secs;
            let next_t = if cur + 1 < self.lines.len() {
                self.lines[cur + 1].time_secs
            } else {
                start_t + 5.0
            };

            let gap = (next_t - start_t).max(0.5);

            // Calculate weight and natural vocal duration of the line
            let words: Vec<&str> = self.lines[cur].text.split_whitespace().collect();
            let total_chars: usize = words.iter().map(|w| w.chars().count()).sum();

            // Natural speaking/singing duration: approx 0.26s per character + word pauses
            let estimated_vocal_dur = (total_chars as f64 * 0.26 + words.len() as f64 * 0.15).max(1.2);
            let vocal_dur = estimated_vocal_dur.min(gap * 0.85);

            let elapsed_in_line = (elapsed_secs - start_t).max(0.0);

            // Instrumental pause check:
            // When line text is empty (LRC instrumental break marker) OR gap between lines is long
            // and singing of current line has finished, switch to InstrumentalBreak until next line arrives!
            let is_empty_line = self.lines[cur].text.trim().is_empty();
            let is_long_pause = gap >= 5.0 && elapsed_in_line >= vocal_dur;

            if (is_empty_line || is_long_pause) && cur + 1 < self.lines.len() {
                // Keep showing instrumental break until 0.5s before next line starts
                if elapsed_secs < (next_t - 0.4) {
                    let prev_idx = if is_empty_line { cur.saturating_sub(1) } else { cur };
                    return ActiveLyricStatus::InstrumentalBreak {
                        prev_line_idx: prev_idx,
                        next_line_idx: cur + 1,
                    };
                }
            }

            if words.is_empty() {
                return ActiveLyricStatus::Singing { line_idx: cur, word_idx: 0 };
            }

            // Word-by-word progress calculation using syllable weights
            let weights: Vec<f64> = words
                .iter()
                .map(|w| (w.chars().count() as f64).max(1.0).powf(0.85))
                .collect();
            let total_weight: f64 = weights.iter().sum();

            let progress = (elapsed_in_line / vocal_dur).clamp(0.0, 1.0);
            let target_accum = progress * total_weight;

            let mut running_weight = 0.0;
            let mut active_word = words.len().saturating_sub(1);
            for (w_i, &w_weight) in weights.iter().enumerate() {
                running_weight += w_weight;
                if running_weight >= target_accum {
                    active_word = w_i;
                    break;
                }
            }

            if progress >= 1.0 {
                active_word = words.len(); // all words in current line finished
            }

            ActiveLyricStatus::Singing { line_idx: cur, word_idx: active_word }
        } else if !self.plain_lines.is_empty() {
            let ratio = if total_duration > 0.0 {
                (elapsed_secs / total_duration).clamp(0.0, 0.999)
            } else {
                0.0
            };
            let float_idx = ratio * (self.plain_lines.len() as f64);
            let cur = (float_idx.floor() as usize).min(self.plain_lines.len().saturating_sub(1));
            ActiveLyricStatus::Singing { line_idx: cur, word_idx: 0 }
        } else {
            ActiveLyricStatus::AllFinished
        }
    }

    /// Returns list of (relative_offset, text, is_active) centered around active_idx
    pub fn get_visible_window(&self, active_idx: usize, num_lines_above: usize, num_lines_below: usize) -> Vec<(i32, String, bool)> {
        let all_lines: Vec<String> = if self.has_synced {
            self.lines.iter().map(|l| l.text.clone()).collect()
        } else {
            self.plain_lines.clone()
        };

        if all_lines.is_empty() {
            return Vec::new();
        }

        let start = active_idx.saturating_sub(num_lines_above);
        let end = (active_idx + num_lines_below + 1).min(all_lines.len());

        let mut res = Vec::new();
        for i in start..end {
            let offset = (i as i32) - (active_idx as i32);
            let is_active = i == active_idx;
            res.push((offset, all_lines[i].clone(), is_active));
        }

        res
    }
}

pub struct LyricsClient {
    client: reqwest::Client,
}

impl LyricsClient {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .user_agent("FringeMusicPlayer/1.0")
                .timeout(std::time::Duration::from_secs(5))
                .build()
                .unwrap_or_default(),
        }
    }

    pub async fn fetch_lyrics(&self, artist: &str, title: &str, target_duration_secs: Option<f64>) -> Result<LyricsData> {
        let clean_title = Regex::new(r"(?i)\(feat\..*?\)|\[feat\..*?\]|\(with.*?\)")
            .unwrap()
            .replace_all(title, "")
            .trim()
            .to_string();

        let clean_artist = artist
            .replace('\u{a0}', " ")
            .split(',')
            .next()
            .unwrap_or(artist)
            .trim()
            .to_string();

        // 1. Try search with full artist & title query
        let query = format!("{} {}", clean_artist, clean_title);
        let search_url = format!(
            "https://lrclib.net/api/search?q={}",
            urlencoding::encode(&query)
        );

        if let Ok(resp) = self.client.get(&search_url).send().await {
            if resp.status().is_success() {
                if let Ok(results) = resp.json::<Value>().await {
                    if let Some(arr) = results.as_array() {
                        if let Some(lyrics) = Self::pick_best_candidate(arr, target_duration_secs, &clean_title) {
                            return Ok(lyrics);
                        }
                    }
                }
            }
        }

        // 2. Fallback search with simplified track title (stripping remaster/deluxe suffixes)
        let simplified_title = Regex::new(r"(?i)\s*[-–(].*?(remaster|version|edition|deluxe|mix|live|mono|stereo|bonus|expanded).*?[)]?")
            .unwrap()
            .replace_all(&clean_title, "")
            .trim()
            .to_string();

        if !simplified_title.is_empty() && simplified_title != clean_title {
            let sub_query = format!("{} {}", clean_artist, simplified_title);
            let sub_search_url = format!(
                "https://lrclib.net/api/search?q={}",
                urlencoding::encode(&sub_query)
            );

            if let Ok(resp) = self.client.get(&sub_search_url).send().await {
                if resp.status().is_success() {
                    if let Ok(results) = resp.json::<Value>().await {
                        if let Some(arr) = results.as_array() {
                            if let Some(lyrics) = Self::pick_best_candidate(arr, target_duration_secs, &simplified_title) {
                                return Ok(lyrics);
                            }
                        }
                    }
                }
            }
        }

        // 3. Direct GET API as fallback
        let get_url = format!(
            "https://lrclib.net/api/get?artist_name={}&track_name={}",
            urlencoding::encode(&clean_artist),
            urlencoding::encode(&clean_title)
        );

        let resp = self.client.get(&get_url).send().await?;
        if !resp.status().is_success() {
            return Err(anyhow!("LRCLIB API status {}", resp.status()));
        }

        let val: Value = resp.json().await?;
        Self::parse_lrclib_json(&val).ok_or_else(|| anyhow!("No lyrics found for {} - {}", artist, title))
    }

    fn pick_best_candidate(items: &[Value], target_duration: Option<f64>, target_title: &str) -> Option<LyricsData> {
        let mut candidates: Vec<(f64, &Value)> = Vec::new();

        let norm_target = target_title.to_lowercase();

        for item in items {
            let synced = item["syncedLyrics"].as_str().unwrap_or("").trim();
            if synced.is_empty() {
                continue;
            }

            let mut score = 0.0;

            // Duration match score
            if let Some(target_dur) = target_duration {
                if let Some(item_dur) = item["duration"].as_f64() {
                    let dur_diff = (item_dur - target_dur).abs();
                    score += dur_diff * 3.0;
                } else {
                    score += 15.0;
                }
            }

            // Title similarity bonus
            let item_name = item["name"].as_str().unwrap_or("").to_lowercase();
            if item_name == norm_target {
                score -= 10.0;
            } else if item_name.contains(&norm_target) {
                score -= 5.0;
            }

            candidates.push((score, item));
        }

        candidates.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());

        for (_, item) in candidates {
            if let Some(data) = Self::parse_lrclib_json(item) {
                return Some(data);
            }
        }

        // Fallback: plain lyrics
        for item in items {
            if let Some(data) = Self::parse_lrclib_json(item) {
                return Some(data);
            }
        }

        None
    }

    fn parse_lrclib_json(val: &Value) -> Option<LyricsData> {
        let synced_raw = val["syncedLyrics"].as_str();
        let plain_raw = val["plainLyrics"].as_str();

        if let Some(synced) = synced_raw {
            if !synced.trim().is_empty() {
                let lines = Self::parse_lrc(synced);
                if !lines.is_empty() {
                    return Some(LyricsData {
                        lines,
                        plain_lines: plain_raw
                            .map(|p| p.lines().map(|s| s.to_string()).collect())
                            .unwrap_or_default(),
                        has_synced: true,
                    });
                }
            }
        }

        if let Some(plain) = plain_raw {
            let plain_lines: Vec<String> = plain
                .lines()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            if !plain_lines.is_empty() {
                return Some(LyricsData {
                    lines: Vec::new(),
                    plain_lines,
                    has_synced: false,
                });
            }
        }

        None
    }

    fn parse_lrc(lrc_text: &str) -> Vec<SyncedLine> {
        let re = Regex::new(r"\[(\d{2}):(\d{2})\.(\d{2,3})\](.*)").unwrap();
        let mut lines = Vec::new();

        for line in lrc_text.lines() {
            if let Some(caps) = re.captures(line) {
                let mins: f64 = caps[1].parse().unwrap_or(0.0);
                let secs: f64 = caps[2].parse().unwrap_or(0.0);
                let millis_str = &caps[3];
                let millis: f64 = if millis_str.len() == 2 {
                    millis_str.parse::<f64>().unwrap_or(0.0) * 10.0
                } else {
                    millis_str.parse().unwrap_or(0.0)
                };

                let time_secs = mins * 60.0 + secs + (millis / 1000.0);
                let text = caps[4].trim().to_string();

                lines.push(SyncedLine { time_secs, text });
            }
        }

        lines.sort_by(|a, b| a.time_secs.partial_cmp(&b.time_secs).unwrap());
        lines
    }
}
