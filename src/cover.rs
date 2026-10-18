use anyhow::{anyhow, Result};
use base64::Engine;
use image::imageops::FilterType;
use image::RgbImage;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use serde_json::Value;
use std::env;
use std::io::{stdout, Write};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::runtime::Handle;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphicProtocol {
    Kitty,
    Iterm2,
    Sixel,
    Unsupported,
}

impl GraphicProtocol {
    pub fn detect() -> Self {
        if env::var("KITTY_WINDOW_ID").is_ok()
            || env::var("GHOSTTY_RESOURCES_DIR").is_ok()
            || env::var("WEZTERM_EXECUTABLE").is_ok()
        {
            return GraphicProtocol::Kitty;
        }

        if let Ok(term_program) = env::var("TERM_PROGRAM") {
            let tp = term_program.to_lowercase();
            if tp.contains("iterm") || tp.contains("wezterm") || tp.contains("mintty") {
                return GraphicProtocol::Iterm2;
            }
        }

        if let Ok(term) = env::var("TERM") {
            let t = term.to_lowercase();
            if t.contains("kitty") || t.contains("ghostty") {
                return GraphicProtocol::Kitty;
            }
            if t.contains("sixel") || t.contains("mlterm") || t.contains("foot") {
                return GraphicProtocol::Sixel;
            }
        }

        GraphicProtocol::Unsupported
    }
}

#[derive(Clone)]
pub struct CoverArtRenderer {
    current_image: Arc<Mutex<Option<RgbImage>>>,
    client: reqwest::Client,
    loading: Arc<Mutex<bool>>,
    pub protocol: GraphicProtocol,
}

impl CoverArtRenderer {
    pub fn new() -> Self {
        let protocol = GraphicProtocol::detect();
        Self {
            current_image: Arc::new(Mutex::new(None)),
            client: reqwest::Client::builder()
                .user_agent("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
                .timeout(Duration::from_secs(4))
                .build()
                .unwrap_or_default(),
            loading: Arc::new(Mutex::new(false)),
            protocol,
        }
    }

    pub fn is_loading(&self) -> bool {
        *self.loading.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Smart cover loader with multi-source fallback (Spotify Direct -> Spotify oEmbed -> iTunes -> YouTube)
    pub fn load_cover_smart(
        &self,
        direct_url: Option<String>,
        spotify_uri: Option<String>,
        query: String,
        handle: &Handle,
    ) {
        let current_image = Arc::clone(&self.current_image);
        let loading = Arc::clone(&self.loading);
        let client = self.client.clone();

        handle.spawn(async move {
            if let Ok(mut l) = loading.lock() {
                *l = true;
            }

            let mut fetched_img = None;

            // 1. Try direct Spotify cover URL
            if let Some(ref url) = direct_url {
                if let Ok(img) = Self::fetch_and_decode(&client, url).await {
                    fetched_img = Some(img);
                }
            }

            // 2. Try Spotify oEmbed via URI
            if fetched_img.is_none() {
                if let Some(ref uri) = spotify_uri {
                    let parts: Vec<&str> = uri.split(':').collect();
                    if parts.len() >= 3 {
                        let entity_type = parts[1];
                        let entity_id = parts[2];
                        let web_url = format!("https://open.spotify.com/{}/{}", entity_type, entity_id);
                        let oembed_url = format!("https://open.spotify.com/oembed?url={}", web_url);

                        if let Ok(resp) = client.get(&oembed_url).send().await {
                            if let Ok(val) = resp.json::<Value>().await {
                                if let Some(thumb) = val["thumbnail_url"].as_str() {
                                    if let Ok(img) = Self::fetch_and_decode(&client, thumb).await {
                                        fetched_img = Some(img);
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // 3. Fallback: Search iTunes Store API
            if fetched_img.is_none() {
                let itunes_url = format!(
                    "https://itunes.apple.com/search?term={}&media=music&entity=song&limit=1",
                    urlencoding::encode(&query)
                );
                if let Ok(resp) = client.get(&itunes_url).send().await {
                    if let Ok(val) = resp.json::<Value>().await {
                        if let Some(results) = val["results"].as_array() {
                            if let Some(first) = results.first() {
                                if let Some(artwork) = first["artworkUrl100"].as_str() {
                                    let hi_res = artwork.replace("100x100bb", "600x600bb");
                                    if let Ok(img) = Self::fetch_and_decode(&client, &hi_res).await {
                                        fetched_img = Some(img);
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // 4. Fallback: yt-dlp thumbnail extract
            if fetched_img.is_none() {
                if let Ok(thumb_url) = Self::fetch_youtube_thumbnail(&query).await {
                    if let Ok(img) = Self::fetch_and_decode(&client, &thumb_url).await {
                        fetched_img = Some(img);
                    }
                }
            }

            if let Ok(mut lock) = current_image.lock() {
                *lock = fetched_img;
            }
            if let Ok(mut l) = loading.lock() {
                *l = false;
            }
        });
    }

    async fn fetch_and_decode(client: &reqwest::Client, url: &str) -> Result<RgbImage> {
        let resp = client.get(url).send().await?;
        let bytes = resp.bytes().await?;
        let img = image::load_from_memory(&bytes)?;
        Ok(img.to_rgb8())
    }

    async fn fetch_youtube_thumbnail(query: &str) -> Result<String> {
        let output = tokio::process::Command::new("yt-dlp")
            .args(&[
                "--get-thumbnail",
                "--no-playlist",
                "--no-warnings",
                &format!("ytsearch1:{}", query),
            ])
            .output()
            .await?;

        if !output.status.success() {
            return Err(anyhow!("yt-dlp get-thumbnail failed"));
        }

        let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if url.starts_with("http") {
            Ok(url)
        } else {
            Err(anyhow!("Invalid thumbnail url"))
        }
    }

    /// Clears any lingering terminal graphics (Kitty, iTerm2)
    pub fn clear_graphics() {
        // Kitty graphics clear all images escape
        let _ = write!(stdout(), "\x1b_Ga=d,d=a\x1b\\");
        let _ = stdout().flush();
    }

    /// Renders cover art.
    /// If terminal supports real graphics (Kitty/iTerm2/Sixel), uses escape codes;
    /// Otherwise renders high-fidelity half-block unicode cells ('▀' with truecolor top/bottom pixels).
    pub fn render(&self, area: Rect, buf: &mut Buffer) {
        if area.width < 2 || area.height < 2 {
            return;
        }

        let guard = self.current_image.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(ref img) = *guard {
            // Attempt native terminal image protocol output (Kitty / iTerm2) strictly inside its designated cell box
            if self.protocol == GraphicProtocol::Kitty {
                Self::render_kitty(img, area);
            } else if self.protocol == GraphicProtocol::Iterm2 {
                Self::render_iterm2(img, area);
            }

            // Universal high-res half-block rendering (always sets the cell grid to match exactly)
            let max_w = area.width as usize;
            let max_h = area.height as usize;
            let size_w = max_w.min(max_h * 2);
            let size_h = size_w / 2;

            if size_w == 0 || size_h == 0 {
                return;
            }

            let start_x = area.x + ((area.width as usize).saturating_sub(size_w) / 2) as u16;
            let start_y = area.y + ((area.height as usize).saturating_sub(size_h) / 2) as u16;

            let target_px_w = size_w as u32;
            let target_px_h = (size_h * 2) as u32;

            let resized = image::imageops::resize(img, target_px_w, target_px_h, FilterType::Triangle);

            for cy in 0..size_h {
                let py_top = (cy * 2) as u32;
                let py_bot = (cy * 2 + 1) as u32;
                let cell_y = start_y + (cy as u16);

                for cx in 0..size_w {
                    let cell_x = start_x + (cx as u16);

                    let top_pixel = resized.get_pixel(cx as u32, py_top);
                    let bot_pixel = if py_bot < target_px_h {
                        resized.get_pixel(cx as u32, py_bot)
                    } else {
                        top_pixel
                    };

                    let fg = Color::Rgb(top_pixel[0], top_pixel[1], top_pixel[2]);
                    let bg = Color::Rgb(bot_pixel[0], bot_pixel[1], bot_pixel[2]);

                    let cell = &mut buf[(cell_x, cell_y)];
                    cell.set_char('▀');
                    cell.set_style(Style::default().fg(fg).bg(bg));
                }
            }
        } else {
            self.render_placeholder(area, buf);
        }
    }

    fn render_placeholder(&self, area: Rect, buf: &mut Buffer) {
        let is_loading = self.is_loading();
        let text = if is_loading { "Loading cover..." } else { "♪ No Cover ♪" };
        let start_x = area.x + (area.width.saturating_sub(text.len() as u16)) / 2;
        let start_y = area.y + (area.height / 2);

        for (j, ch) in text.chars().enumerate() {
            let col = start_x + (j as u16);
            if col < area.x + area.width && start_y < area.y + area.height {
                let cell = &mut buf[(col, start_y)];
                cell.set_char(ch);
                cell.set_style(Style::default().fg(Color::Rgb(140, 150, 175)));
            }
        }
    }

    fn render_kitty(img: &RgbImage, area: Rect) {
        let mut png_bytes = Vec::new();
        let dynamic = image::DynamicImage::ImageRgb8(img.clone());
        if dynamic.write_to(&mut std::io::Cursor::new(&mut png_bytes), image::ImageFormat::Png).is_err() {
            return;
        }

        let b64 = base64::engine::general_purpose::STANDARD.encode(&png_bytes);
        // Kitty graphics escape sequence at (area.x, area.y)
        let _ = write!(
            stdout(),
            "\x1b[{};{}H\x1b_Ga=T,f=100,c={},r={};{}\x1b\\",
            area.y + 1,
            area.x + 1,
            area.width,
            area.height,
            b64
        );
        let _ = stdout().flush();
    }

    fn render_iterm2(img: &RgbImage, area: Rect) {
        let mut png_bytes = Vec::new();
        let dynamic = image::DynamicImage::ImageRgb8(img.clone());
        if dynamic.write_to(&mut std::io::Cursor::new(&mut png_bytes), image::ImageFormat::Png).is_err() {
            return;
        }

        let b64 = base64::engine::general_purpose::STANDARD.encode(&png_bytes);
        // iTerm2 inline image escape sequence
        let _ = write!(
            stdout(),
            "\x1b[{};{}H\x1b]1337;File=inline=1;width={}cells;height={}cells;preserveAspectRatio=1:{}\x07",
            area.y + 1,
            area.x + 1,
            area.width,
            area.height,
            b64
        );
        let _ = stdout().flush();
    }
}
