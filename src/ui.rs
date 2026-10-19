use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Borders, Clear, Gauge, Paragraph, Row, Table,
};
use ratatui::Frame;

use crate::app::{AfkMode, App};
use crate::spotify::PRESETS;

pub fn render(frame: &mut Frame, app: &mut App) {
    let size = frame.area();

    // If AFK / Screensaver is active, render fullscreen AFK view
    if app.is_afk {
        render_afk_mode(frame, app, size);
        return;
    }

    // Main layout: Header (3) -> Center Body (Min 10) -> Visualizer (10) -> Footer (1)
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(12),
            Constraint::Length(10),
            Constraint::Length(1),
        ])
        .split(size);

    render_header(frame, app, chunks[0]);
    render_body(frame, app, chunks[1]);
    render_visualizer(frame, app, chunks[2]);
    render_footer(frame, app, chunks[3]);

    if app.is_url_modal_open {
        render_url_modal(frame, app, size);
    }
}

fn render_afk_mode(frame: &mut Frame, app: &mut App, area: Rect) {
    // Never show cover graphics over fullscreen lyrics / screensaver
    crate::cover::CoverArtRenderer::clear_graphics();
    match app.afk_mode {
        AfkMode::Lyrics => render_afk_lyrics(frame, app, area),
        AfkMode::Spectrum => render_afk_spectrum(frame, app, area),
    }
}

fn render_afk_lyrics(frame: &mut Frame, app: &App, area: Rect) {
    let afk_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::Rgb(60, 90, 130)))
        .style(Style::default().bg(Color::Rgb(12, 16, 24)));

    let inner = afk_block.inner(area);
    frame.render_widget(afk_block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Top track info & elapsed
            Constraint::Min(8),    // Full-height Scrolling Karaoke Lyrics
            Constraint::Length(2), // Bottom switch hints
        ])
        .split(inner);

    // 1. Top Track Info & Progress
    let (title, artist, total_duration) = match app.current_track() {
        Some(t) => (t.title.as_str(), t.artist.as_str(), (t.duration_ms as f64) / 1000.0),
        None => ("No Track Playing", "", 0.0),
    };

    let elapsed = app.playback_elapsed();
    let elapsed_mins = (elapsed as u64) / 60;
    let elapsed_secs = (elapsed as u64) % 60;
    let total_mins = (total_duration as u64) / 60;
    let total_secs = (total_duration as u64) % 60;

    let top_lines = vec![
        Line::from(vec![
            Span::styled(" ♫ ", Style::default().fg(Color::Cyan)),
            Span::styled(title, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
            Span::styled(format!("  ·  {}", artist), Style::default().fg(Color::Rgb(240, 210, 100))),
            Span::styled(
                format!("    {:02}:{:02} / {:02}:{:02}", elapsed_mins, elapsed_secs, total_mins, total_secs),
                Style::default().fg(Color::Rgb(160, 180, 200)),
            ),
        ]),
    ];
    let top_p = Paragraph::new(top_lines).alignment(Alignment::Center);
    frame.render_widget(top_p, chunks[0]);

    // 2. Full-height Scrolling Karaoke Lyrics (Centered on current singing line, NO duplication)
    let lyrics_height = chunks[1].height as usize;
    let half_height = lyrics_height / 2;

    if let Some(ref lyrics) = app.lyrics {
        let status = lyrics.get_active_status(elapsed, total_duration);
        let cfg = &app.config;
        let mut rendered_lines: Vec<Line> = Vec::new();

        match status {
            crate::lyrics::ActiveLyricStatus::Intro => {
                // Before vocal begins: Show aesthetic INTRO indicator and upcoming lines
                let window = lyrics.get_visible_window(0, 0, half_height.saturating_sub(1));
                rendered_lines.push(Line::from(vec![
                    Span::styled("♪  INTRO  ♪", Style::default().fg(cfg.color_primary).add_modifier(Modifier::BOLD)),
                ]));
                rendered_lines.push(Line::from(""));
                for (_, line_text, _) in window {
                    rendered_lines.push(Line::from(Span::styled(
                        line_text,
                        Style::default().fg(cfg.color_inactive),
                    )));
                }
            }
            crate::lyrics::ActiveLyricStatus::InstrumentalBreak { prev_line_idx: _, next_line_idx } => {
                // Show last sung line (dimmed), aesthetic break badge, and upcoming lines
                let window = lyrics.get_visible_window(next_line_idx, 1, half_height);
                for (offset, line_text, _) in window {
                    if offset < 0 {
                        rendered_lines.push(Line::from(Span::styled(
                            line_text,
                            Style::default().fg(cfg.color_sung_word),
                        )));
                    } else if offset == 0 {
                        rendered_lines.push(Line::from(""));
                        rendered_lines.push(Line::from(vec![
                            Span::styled("♪  ~ INSTRUMENTAL ~  ♪", Style::default().fg(cfg.color_accent).add_modifier(Modifier::BOLD)),
                        ]));
                        rendered_lines.push(Line::from(""));
                        rendered_lines.push(Line::from(Span::styled(
                            line_text,
                            Style::default().fg(cfg.color_primary),
                        )));
                    } else {
                        rendered_lines.push(Line::from(Span::styled(
                            line_text,
                            Style::default().fg(cfg.color_inactive),
                        )));
                    }
                }
            }
            crate::lyrics::ActiveLyricStatus::Singing { line_idx, word_idx } => {
                let window = lyrics.get_visible_window(line_idx, half_height.saturating_sub(1), half_height);

                for (offset, line_text, is_active) in window {
                    if is_active {
                        let words: Vec<&str> = line_text.split_whitespace().collect();
                        if words.is_empty() {
                            rendered_lines.push(Line::from(""));
                            continue;
                        }

                        let total_words = words.len();
                        let active_w = word_idx.min(total_words);

                        let mut spans = vec![
                            Span::styled("▶  ", Style::default().fg(cfg.color_accent).add_modifier(Modifier::BOLD)),
                        ];

                        for (w_i, word) in words.iter().enumerate() {
                            if w_i < active_w {
                                // Already vocalized word
                                spans.push(Span::styled(
                                    format!("{} ", word),
                                    Style::default()
                                        .fg(cfg.color_sung_word)
                                        .add_modifier(Modifier::BOLD),
                                ));
                            } else if w_i == active_w && active_w < total_words {
                                // Currently vocalized word: Adaptive effects based on config
                                match cfg.lyric_effect.as_str() {
                                    "box" => {
                                        spans.push(Span::styled(
                                            format!("[{}] ", word),
                                            Style::default()
                                                .fg(cfg.color_active_word)
                                                .add_modifier(Modifier::BOLD),
                                        ));
                                    }
                                    "dots" => {
                                        spans.push(Span::styled(
                                            format!("·{}· ", word),
                                            Style::default()
                                                .fg(cfg.color_active_word)
                                                .add_modifier(Modifier::BOLD),
                                        ));
                                    }
                                    "underline" => {
                                        // Precise adaptive underline matching word bounds
                                        spans.push(Span::styled(
                                            format!("{}", word),
                                            Style::default()
                                                .fg(cfg.color_active_word)
                                                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
                                        ));
                                        spans.push(Span::raw(" "));
                                    }
                                    "bold" => {
                                        spans.push(Span::styled(
                                            format!("{} ", word),
                                            Style::default()
                                                .fg(cfg.color_active_word)
                                                .add_modifier(Modifier::BOLD),
                                        ));
                                    }
                                    _ => {
                                        // "glow" (Default): Glowing bright foreground with inverted focus or underline
                                        spans.push(Span::styled(
                                            format!("{}", word),
                                            Style::default()
                                                .fg(cfg.color_active_word)
                                                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
                                        ));
                                        spans.push(Span::raw(" "));
                                    }
                                }
                            } else {
                                // Upcoming words: Muted clean foreground
                                spans.push(Span::styled(
                                    format!("{} ", word),
                                    Style::default()
                                        .fg(cfg.color_primary)
                                        .add_modifier(Modifier::empty()),
                                ));
                            }
                        }

                        spans.push(Span::styled(" ◀", Style::default().fg(cfg.color_accent).add_modifier(Modifier::BOLD)));
                        rendered_lines.push(Line::from(spans));
                    } else {
                        // Surrounding lines with smooth distance fade
                        let color = match offset.abs() {
                            1 => cfg.color_sung_word,
                            2 => cfg.color_inactive,
                            _ => {
                                // Dimmer fade
                                match cfg.color_inactive {
                                    Color::Rgb(r, g, b) => Color::Rgb(r / 2, g / 2, b / 2),
                                    _ => Color::DarkGray,
                                }
                            }
                        };

                        rendered_lines.push(Line::from(Span::styled(
                            line_text,
                            Style::default().fg(color),
                        )));
                    }
                }
            }
            crate::lyrics::ActiveLyricStatus::AllFinished => {
                let window = lyrics.get_visible_window(lyrics.lines.len().saturating_sub(1), half_height, 0);
                for (_, line_text, _) in window {
                    rendered_lines.push(Line::from(Span::styled(
                        line_text,
                        Style::default().fg(cfg.color_inactive),
                    )));
                }
                rendered_lines.push(Line::from(""));
                rendered_lines.push(Line::from(Span::styled(
                    "♪  OUTRO  ♪",
                    Style::default().fg(cfg.color_accent).add_modifier(Modifier::BOLD),
                )));
            }
        }

        let lyrics_p = Paragraph::new(rendered_lines).alignment(Alignment::Center);
        frame.render_widget(lyrics_p, chunks[1]);
    } else {
        let msg = if app.is_loading_lyrics {
            "Lyrics loading..."
        } else {
            "♪  INSTRUMENTAL  ♪"
        };
        let empty_p = Paragraph::new(vec![
            Line::from(""),
            Line::from(""),
            Line::from(Span::styled(msg, Style::default().fg(Color::Rgb(100, 130, 170)))),
        ])
        .alignment(Alignment::Center);
        frame.render_widget(empty_p, chunks[1]);
    }

    // 3. Bottom Hints
    let bottom_line = Line::from(vec![
        Span::styled(" [◀ / ▶] ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::styled("Switch to Fullscreen Spectrum  ·  ", Style::default().fg(Color::Rgb(140, 160, 190))),
        Span::styled("[Any Key] ", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        Span::styled("Exit AFK Screensaver", Style::default().fg(Color::Rgb(140, 160, 190))),
    ]);
    let bottom_p = Paragraph::new(bottom_line).alignment(Alignment::Center);
    frame.render_widget(bottom_p, chunks[2]);
}

fn render_afk_spectrum(frame: &mut Frame, app: &mut App, area: Rect) {
    let afk_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(" Fullscreen Spectrum Visualizer ")
        .title_style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
        .border_style(Style::default().fg(Color::Rgb(60, 90, 130)))
        .style(Style::default().bg(Color::Rgb(12, 16, 24)));

    let inner = afk_block.inner(area);
    frame.render_widget(afk_block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2), // Track title
            Constraint::Min(8),    // Fullscreen CAVA bars
            Constraint::Length(1), // Hint
        ])
        .split(inner);

    if let Some(t) = app.current_track() {
        let p = Paragraph::new(Line::from(vec![
            Span::styled("♪ ", Style::default().fg(Color::Cyan)),
            Span::styled(&t.title, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
            Span::styled(format!("  —  {}", t.artist), Style::default().fg(Color::Rgb(240, 210, 100))),
        ]))
        .alignment(Alignment::Center);
        frame.render_widget(p, chunks[0]);
    }

    if let Some(ref mut ext) = app.external_vis {
        ext.render(chunks[1], frame.buffer_mut());
    } else {
        let vis_buffer = app.audio_player.visualizer();
        let cfg = app.config.clone();
        app.visualizer.render(chunks[1], frame.buffer_mut(), &vis_buffer, &cfg);
    }

    let bottom_line = Line::from(vec![
        Span::styled(" [◀ / ▶] ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::styled("Switch to Fullscreen Lyrics  ·  ", Style::default().fg(Color::Rgb(140, 160, 190))),
        Span::styled("[Any Key] ", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        Span::styled("Exit AFK Screensaver", Style::default().fg(Color::Rgb(140, 160, 190))),
    ]);
    let bottom_p = Paragraph::new(bottom_line).alignment(Alignment::Center);
    frame.render_widget(bottom_p, chunks[2]);
}

fn render_header(frame: &mut Frame, app: &mut App, area: Rect) {
    let cfg = &app.config;
    let header_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(cfg.color_inactive));

    let inner = header_block.inner(area);
    frame.render_widget(header_block, area);

    let is_playing = app.audio_player.is_playing.load(std::sync::atomic::Ordering::Relaxed);
    let is_paused = app.audio_player.is_paused();

    let (status_badge, badge_color) = if app.is_loading_track {
        ("⟳ Loading", cfg.color_sung_word)
    } else if is_playing && !is_paused {
        ("▶ Playing", cfg.color_accent)
    } else if is_playing && is_paused {
        ("⏸ Paused", cfg.color_sung_word)
    } else {
        ("⏹ Stopped", cfg.color_inactive)
    };

    let vol_f = app.audio_player.get_volume().clamp(0.0, 1.0);
    let vol_pct = (vol_f * 100.0).round() as u32;

    // Header layout: Title/Status (min 22) -> Controls (18) -> Vol Slider Mixer (28)
    let header_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min(24),   // Logo & Status
            Constraint::Length(24), // Quick Click Controls [⏮ ⏯ ⏭ 🔀 🔁]
            Constraint::Length(28), // Volume Mixer [ 🔉 ━━━━━●━━━ 85% ]
        ])
        .split(inner);

    // 1. Logo & Status
    let logo_spans = vec![
        Span::styled("♫ FRINGE ", Style::default().fg(cfg.color_primary).add_modifier(Modifier::BOLD)),
        Span::styled(format!("· {} ", status_badge), Style::default().fg(badge_color).add_modifier(Modifier::BOLD)),
    ];
    frame.render_widget(Paragraph::new(Line::from(logo_spans)), header_chunks[0]);

    // 2. Clickable media controls
    let ctrl_area = header_chunks[1];
    let ctrl_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(4), // Prev [⏮]
            Constraint::Length(5), // Play/Pause [⏯]
            Constraint::Length(4), // Next [⏭]
            Constraint::Length(5), // Shuffle [🔀]
            Constraint::Length(5), // Repeat [🔁]
        ])
        .split(ctrl_area);

    app.rects.prev_btn = Some(ctrl_chunks[0]);
    app.rects.play_pause_btn = Some(ctrl_chunks[1]);
    app.rects.next_btn = Some(ctrl_chunks[2]);
    app.rects.shuffle_btn = Some(ctrl_chunks[3]);
    app.rects.repeat_btn = Some(ctrl_chunks[4]);

    frame.render_widget(
        Paragraph::new(Span::styled("⏮ ", Style::default().fg(cfg.color_primary).add_modifier(Modifier::BOLD))),
        ctrl_chunks[0],
    );
    let pp_icon = if is_playing && !is_paused { "⏸ " } else { "▶ " };
    frame.render_widget(
        Paragraph::new(Span::styled(pp_icon, Style::default().fg(badge_color).add_modifier(Modifier::BOLD))),
        ctrl_chunks[1],
    );
    frame.render_widget(
        Paragraph::new(Span::styled("⏭ ", Style::default().fg(cfg.color_primary).add_modifier(Modifier::BOLD))),
        ctrl_chunks[2],
    );

    let shuff_color = if app.shuffle { cfg.color_accent } else { cfg.color_inactive };
    frame.render_widget(
        Paragraph::new(Span::styled("🔀 ", Style::default().fg(shuff_color).add_modifier(if app.shuffle { Modifier::BOLD } else { Modifier::empty() }))),
        ctrl_chunks[3],
    );

    let rep_color = if app.repeat { cfg.color_accent } else { cfg.color_inactive };
    frame.render_widget(
        Paragraph::new(Span::styled("🔁 ", Style::default().fg(rep_color).add_modifier(if app.repeat { Modifier::BOLD } else { Modifier::empty() }))),
        ctrl_chunks[4],
    );

    // 3. Interactive Volume Mixer Slider
    let vol_area = header_chunks[2];
    app.rects.volume_slider = Some(vol_area);

    // Draw visual volume slider: "🔉 ━━━━━●━━━ 85%"
    let slider_width = 10;
    let filled = ((vol_f * (slider_width as f32)).round() as usize).min(slider_width);
    let empty = slider_width.saturating_sub(filled);

    let vol_icon = if vol_pct == 0 { "🔇" } else if vol_pct < 50 { "🔉" } else { "🔊" };
    let mut bar_str = String::with_capacity(slider_width + 1);
    for _ in 0..filled {
        bar_str.push('━');
    }
    bar_str.push('●');
    for _ in 0..empty.saturating_sub(1) {
        bar_str.push('─');
    }

    let vol_spans = vec![
        Span::styled(format!("{} ", vol_icon), Style::default()),
        Span::styled(bar_str, Style::default().fg(cfg.color_accent).add_modifier(Modifier::BOLD)),
        Span::styled(format!(" {:>3}%", vol_pct), Style::default().fg(cfg.color_sung_word)),
    ];
    frame.render_widget(Paragraph::new(Line::from(vol_spans)).alignment(Alignment::Right), vol_area);
}

fn render_body(frame: &mut Frame, app: &mut App, area: Rect) {
    let body_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(38),
            Constraint::Min(40),
        ])
        .split(area);

    render_left_panel(frame, app, body_chunks[0]);
    render_right_panel(frame, app, body_chunks[1]);
}

fn render_left_panel(frame: &mut Frame, app: &mut App, area: Rect) {
    let left_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(14), // Cover art box
            Constraint::Min(8),     // Track info & progress
        ])
        .split(area);

    let cfg = &app.config;
    // 1. Cover Art Box
    let cover_title = match app.cover_renderer.protocol {
        crate::cover::GraphicProtocol::Kitty => " Album Cover (Kitty Graphics) ",
        crate::cover::GraphicProtocol::Iterm2 => " Album Cover (iTerm2 Graphics) ",
        crate::cover::GraphicProtocol::Sixel => " Album Cover (Sixel Graphics) ",
        crate::cover::GraphicProtocol::Unsupported => " Album Cover [⚠ Unicode fallback: term lacks image protocol] ",
    };

    let cover_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(cover_title)
        .title_style(Style::default().fg(cfg.color_primary).add_modifier(Modifier::BOLD))
        .border_style(Style::default().fg(cfg.color_inactive));

    let inner_cover = cover_block.inner(left_chunks[0]);
    frame.render_widget(cover_block, left_chunks[0]);
    app.cover_renderer.render(inner_cover, frame.buffer_mut());

    // 2. Track Info & Progress
    let info_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(" Now Playing ")
        .title_style(Style::default().fg(cfg.color_primary).add_modifier(Modifier::BOLD))
        .border_style(Style::default().fg(cfg.color_inactive));

    let inner_info = info_block.inner(left_chunks[1]);
    frame.render_widget(info_block, left_chunks[1]);

    let (title, artist, pl_name, total_duration) = match app.current_track() {
        Some(t) => (
            t.title.as_str(),
            t.artist.as_str(),
            app.playlist.as_ref().map(|p| p.title.as_str()).unwrap_or("Spotify"),
            (t.duration_ms as f64) / 1000.0,
        ),
        None => ("No Track Playing", "Select a track to start", "Playlist", 0.0),
    };

    let elapsed = app.playback_elapsed();
    let ratio = if total_duration > 0.0 {
        (elapsed / total_duration).clamp(0.0, 1.0)
    } else {
        0.0
    };

    let elapsed_mins = (elapsed as u64) / 60;
    let elapsed_secs = (elapsed as u64) % 60;
    let total_mins = (total_duration as u64) / 60;
    let total_secs = (total_duration as u64) % 60;
    let time_str = format!(
        "{:02}:{:02} / {:02}:{:02}",
        elapsed_mins, elapsed_secs, total_mins, total_secs
    );

    let info_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // Title
            Constraint::Length(1), // Artist
            Constraint::Length(1), // Playlist
            Constraint::Length(1), // Spacing
            Constraint::Length(1), // Progress bar
            Constraint::Length(1), // Time label
            Constraint::Min(1),    // Status msg
        ])
        .split(inner_info);

    let title_line = Paragraph::new(Line::from(vec![
        Span::styled("♪ ", Style::default().fg(Color::Rgb(0, 220, 255))),
        Span::styled(title, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
    ]));
    frame.render_widget(title_line, info_layout[0]);

    let artist_line = Paragraph::new(Line::from(vec![
        Span::styled("  ", Style::default()),
        Span::styled(artist, Style::default().fg(Color::Rgb(240, 210, 100))),
    ]));
    frame.render_widget(artist_line, info_layout[1]);

    let pl_line = Paragraph::new(Line::from(vec![
        Span::styled("  ", Style::default()),
        Span::styled(format!("📁 {}", pl_name), Style::default().fg(Color::Rgb(140, 150, 170))),
    ]));
    frame.render_widget(pl_line, info_layout[2]);

    let gauge = Gauge::default()
        .gauge_style(
            Style::default()
                .fg(cfg.color_accent)
                .bg(Color::Rgb(35, 35, 35)),
        )
        .ratio(ratio)
        .label("");
    app.rects.progress_bar = Some(info_layout[4]);
    frame.render_widget(gauge, info_layout[4]);

    let time_label = Paragraph::new(Line::from(vec![
        Span::styled(time_str, Style::default().fg(cfg.color_primary)),
        Span::raw("   "),
        Span::styled(
            format!("({:.0}%)", ratio * 100.0),
            Style::default().fg(cfg.color_inactive),
        ),
    ])).alignment(Alignment::Center);
    frame.render_widget(time_label, info_layout[5]);

    let status_p = Paragraph::new(Line::from(vec![
        Span::styled(&app.status_message, Style::default().fg(cfg.color_sung_word)),
    ])).alignment(Alignment::Center);
    frame.render_widget(status_p, info_layout[6]);
}

fn render_right_panel(frame: &mut Frame, app: &mut App, area: Rect) {
    let cfg = &app.config;
    let pl_title = match &app.playlist {
        Some(p) => format!(" Playlist: {} ({} tracks) ", p.title, p.tracks.len()),
        None => " Playlist Queue (Empty) ".to_string(),
    };

    let panel_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(pl_title)
        .title_style(Style::default().fg(cfg.color_primary).add_modifier(Modifier::BOLD))
        .border_style(Style::default().fg(cfg.color_inactive));

    let inner = panel_block.inner(area);
    frame.render_widget(panel_block, area);

    let panel_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(if app.is_searching { 2 } else { 0 }),
            Constraint::Min(3),
        ])
        .split(inner);

    if app.is_searching {
        let search_text = format!(" 🔍 Search: {}▌", app.search_query);
        let search_p = Paragraph::new(search_text)
            .style(Style::default().fg(Color::Yellow));
        frame.render_widget(search_p, panel_chunks[0]);
    }

    let filtered = app.filtered_indices();
    if filtered.is_empty() {
        let empty_msg = if app.playlist.is_none() {
            vec![
                Line::from(""),
                Line::from(Span::styled(
                    "No playlist loaded yet.",
                    Style::default().fg(Color::Rgb(160, 170, 190)),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "Press 'L' to load a Spotify playlist link or choose a curated preset!",
                    Style::default().fg(Color::Cyan),
                )),
            ]
        } else {
            vec![
                Line::from(""),
                Line::from(Span::styled(
                    "No tracks match your search.",
                    Style::default().fg(Color::Rgb(160, 170, 190)),
                )),
            ]
        };
        let p = Paragraph::new(empty_msg).alignment(Alignment::Center);
        frame.render_widget(p, panel_chunks[1]);
        return;
    }

    let visible_rows = (panel_chunks[1].height.saturating_sub(2)) as usize;
    if visible_rows == 0 {
        return;
    }

    let selected_pos = filtered.iter().position(|&i| i == app.selected_idx).unwrap_or(0);
    if selected_pos < app.list_scroll {
        app.list_scroll = selected_pos;
    } else if selected_pos >= app.list_scroll + visible_rows {
        app.list_scroll = selected_pos.saturating_sub(visible_rows - 1);
    }

    let pl = app.playlist.as_ref().unwrap();

    let header_row = Row::new(vec![
        cell_text(" ", Style::default()),
        cell_text("#", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        cell_text("Title", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        cell_text("Artist", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        cell_text("Time", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
    ])
    .style(Style::default().bg(Color::Rgb(30, 36, 50)))
    .height(1);

    let rows: Vec<Row> = filtered
        .iter()
        .skip(app.list_scroll)
        .take(visible_rows)
        .map(|&idx| {
            let track = &pl.tracks[idx];
            let is_cur = app.current_track_idx == Some(idx);
            let is_sel = app.selected_idx == idx;

            let icon = if is_cur {
                "▶"
            } else if is_sel {
                "●"
            } else {
                " "
            };

            let row_style = if is_cur {
                Style::default()
                    .fg(Color::Rgb(0, 240, 255))
                    .bg(Color::Rgb(25, 45, 65))
                    .add_modifier(Modifier::BOLD)
            } else if is_sel {
                Style::default()
                    .fg(Color::White)
                    .bg(Color::Rgb(40, 50, 70))
            } else {
                Style::default().fg(Color::Rgb(180, 190, 210))
            };

            Row::new(vec![
                cell_text(icon, row_style),
                cell_text(format!("{:02}", idx + 1), row_style),
                cell_text(&track.title, row_style),
                cell_text(&track.artist, row_style),
                cell_text(track.duration_formatted(), row_style),
            ])
            .height(1)
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(2),
            Constraint::Length(4),
            Constraint::Percentage(55),
            Constraint::Percentage(30),
            Constraint::Length(6),
        ],
    )
    .header(header_row);

    app.rects.playlist_table = Some(panel_chunks[1]);
    frame.render_widget(table, panel_chunks[1]);
}

fn cell_text<'a>(text: impl Into<String>, style: Style) -> ratatui::widgets::Cell<'a> {
    ratatui::widgets::Cell::from(text.into()).style(style)
}

fn render_visualizer(frame: &mut Frame, app: &mut App, area: Rect) {
    let cfg = &app.config;
    let title = if app.external_vis.is_some() {
        format!(" Visualizer ({}) ", app.config.visualizer_cmd.split_whitespace().next().unwrap_or("Custom"))
    } else {
        " Spectrum Visualizer ".to_string()
    };

    let vis_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(title)
        .title_style(Style::default().fg(cfg.color_primary).add_modifier(Modifier::BOLD))
        .border_style(Style::default().fg(cfg.color_inactive));

    let inner = vis_block.inner(area);
    frame.render_widget(vis_block, area);

    if let Some(ref mut ext) = app.external_vis {
        ext.render(inner, frame.buffer_mut());
    } else {
        let vis_buffer = app.audio_player.visualizer();
        let cfg = app.config.clone();
        app.visualizer.render(inner, frame.buffer_mut(), &vis_buffer, &cfg);
    }
}

fn render_footer(frame: &mut Frame, _app: &App, area: Rect) {
    let shortcuts = Line::from(vec![
        Span::styled(" [Space]", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::raw(" Play/Pause "),
        Span::styled(" [N]", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::raw(" Next "),
        Span::styled(" [P]", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::raw(" Prev "),
        Span::styled(" [L]", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::raw(" Load URL "),
        Span::styled(" [/]", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::raw(" Search "),
        Span::styled(" [S]", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::raw(" Shuffle "),
        Span::styled(" [R]", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::raw(" Repeat "),
        Span::styled(" [+/-]", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::raw(" Vol "),
        Span::styled(" [Q]", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::raw(" Quit"),
    ]);

    let footer = Paragraph::new(shortcuts).alignment(Alignment::Center);
    frame.render_widget(footer, area);
}

fn render_url_modal(frame: &mut Frame, app: &mut App, area: Rect) {
    let popup_width = 72.min(area.width.saturating_sub(4));
    let popup_height = 19.min(area.height.saturating_sub(2));

    let popup_area = Rect {
        x: area.x + (area.width.saturating_sub(popup_width)) / 2,
        y: area.y + (area.height.saturating_sub(popup_height)) / 2,
        width: popup_width,
        height: popup_height,
    };

    app.rects.modal_area = Some(popup_area);
    frame.render_widget(Clear, popup_area);

    let modal_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .title(" Load Spotify Playlist / Album / Track ")
        .title_style(Style::default().fg(Color::Rgb(0, 240, 255)).add_modifier(Modifier::BOLD))
        .border_style(Style::default().fg(Color::Rgb(0, 200, 240)))
        .style(Style::default().bg(Color::Rgb(20, 24, 34)));

    let inner = modal_block.inner(popup_area);
    frame.render_widget(modal_block, popup_area);

    let modal_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // Hint
            Constraint::Length(3), // URL Input box
            Constraint::Length(1), // Presets label
            Constraint::Min(6),    // Presets list
            Constraint::Length(1), // Buttons hint
        ])
        .split(inner);

    app.rects.modal_presets = Some(modal_chunks[3]);

    let hint = Paragraph::new(
        "Paste any open.spotify.com link (playlist, album, or track) below:"
    ).style(Style::default().fg(Color::Rgb(180, 190, 210)));
    frame.render_widget(hint, modal_chunks[0]);

    let input_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::Cyan));
    let input_p = Paragraph::new(format!("{}▌", app.url_input))
        .block(input_block)
        .style(Style::default().fg(Color::White));
    frame.render_widget(input_p, modal_chunks[1]);

    let presets_label = Paragraph::new(
        "── Or choose from curated playlists (Up/Down arrows to select) ──"
    ).style(Style::default().fg(Color::Rgb(140, 150, 170)));
    frame.render_widget(presets_label, modal_chunks[2]);

    let preset_lines: Vec<Line> = PRESETS
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let is_sel = i == app.active_preset_idx;
            let marker = if is_sel { "▶ " } else { "  " };
            let style = if is_sel {
                Style::default()
                    .fg(Color::Rgb(0, 255, 200))
                    .bg(Color::Rgb(30, 45, 60))
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Rgb(170, 180, 200))
            };

            Line::from(vec![
                Span::styled(marker, style),
                Span::styled(format!("{:<22}", p.name), style),
                Span::styled(format!(" - {}", p.desc), Style::default().fg(Color::DarkGray)),
            ])
        })
        .collect();

    let presets_p = Paragraph::new(preset_lines);
    frame.render_widget(presets_p, modal_chunks[3]);

    let keys_hint = Paragraph::new(
        "[Enter] Load & Play   [Tab] Cycle Presets   [Esc] Cancel"
    )
    .alignment(Alignment::Center)
    .style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD));
    frame.render_widget(keys_hint, modal_chunks[4]);
}
