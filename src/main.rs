mod app;
mod audio;
mod config;
mod cover;
mod external_vis;
mod lyrics;
mod spotify;
mod ui;
mod visualizer;

use anyhow::Result;
use app::AfkMode;
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::io::stdout;
use std::panic;
use std::time::{Duration, Instant};

#[tokio::main]
async fn main() -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    let default_panic = panic::take_hook();
    panic::set_hook(Box::new(move |panic_info| {
        let _ = disable_raw_mode();
        let _ = execute!(std::io::stdout(), LeaveAlternateScreen);
        default_panic(panic_info);
    }));

    let tokio_handle = tokio::runtime::Handle::current();
    let mut app = app::App::new(tokio_handle)?;

    // Open URL input modal immediately at launch
    app.is_url_modal_open = true;

    let mut last_tick = Instant::now();
    let target_fps = app.config.framerate.clamp(15, 144);
    let tick_rate = Duration::from_millis(1000 / target_fps);

    loop {
        app.process_events();

        let now = Instant::now();
        let dt = (now - last_tick).as_secs_f64();
        last_tick = now;
        app.visualizer.update(dt, &app.config);

        terminal.draw(|f| ui::render(f, &mut app))?;

        let timeout = tick_rate.saturating_sub(now.elapsed());
        if event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
                    break;
                }

                // If currently in AFK Screensaver mode
                if app.is_afk {
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Char('Q') => {
                            break;
                        }
                        // Arrow keys switch AFK views between Lyrics and CAVA Spectrum
                        KeyCode::Left | KeyCode::Right | KeyCode::Up | KeyCode::Down => {
                            app.afk_mode = match app.afk_mode {
                                AfkMode::Lyrics => AfkMode::Spectrum,
                                AfkMode::Spectrum => AfkMode::Lyrics,
                            };
                            continue;
                        }
                        _ => {
                            // Any other key wakes up from AFK mode
                            app.is_afk = false;
                            app.last_activity = Instant::now();
                        }
                    }
                } else {
                    app.last_activity = Instant::now();
                }

                // 1. URL Modal open mode
                if app.is_url_modal_open {
                    match key.code {
                        KeyCode::Esc => {
                            app.is_url_modal_open = false;
                        }
                        KeyCode::Enter => {
                            let url = if app.url_input.trim().is_empty() {
                                spotify::PRESETS[app.active_preset_idx].url.to_string()
                            } else {
                                app.url_input.clone()
                            };
                            app.load_spotify_url(&url);
                        }
                        KeyCode::Tab | KeyCode::Down => {
                            app.select_preset_next();
                        }
                        KeyCode::Up => {
                            app.select_preset_prev();
                        }
                        KeyCode::Backspace => {
                            app.url_input.pop();
                        }
                        KeyCode::Char(c) => {
                            app.url_input.push(c);
                        }
                        _ => {}
                    }
                    continue;
                }

                // 2. Search mode
                if app.is_searching {
                    match key.code {
                        KeyCode::Esc | KeyCode::Enter => {
                            app.is_searching = false;
                        }
                        KeyCode::Backspace => {
                            app.search_query.pop();
                        }
                        KeyCode::Char(c) => {
                            app.search_query.push(c);
                        }
                        _ => {}
                    }
                    continue;
                }

                // 3. Normal playback mode
                match key.code {
                    KeyCode::Char('q') | KeyCode::Char('Q') => {
                        break;
                    }
                    KeyCode::Char('l') | KeyCode::Char('L') | KeyCode::Char('o') | KeyCode::Char('O') => {
                        app.is_url_modal_open = true;
                    }
                    KeyCode::Char(' ') => {
                        app.toggle_play_pause();
                    }
                    KeyCode::Char('n') => {
                        app.next_track();
                    }
                    KeyCode::Char('p') => {
                        app.prev_track();
                    }
                    KeyCode::Enter => {
                        app.play_track_at(app.selected_idx);
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        app.select_next();
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        app.select_prev();
                    }
                    KeyCode::PageDown => {
                        for _ in 0..10 {
                            app.select_next();
                        }
                    }
                    KeyCode::PageUp => {
                        for _ in 0..10 {
                            app.select_prev();
                        }
                    }
                    KeyCode::Char('/') => {
                        app.is_searching = true;
                        app.search_query.clear();
                    }
                    KeyCode::Char('s') | KeyCode::Char('S') => {
                        app.shuffle = !app.shuffle;
                        app.status_message = format!("Shuffle: {}", if app.shuffle { "ON" } else { "OFF" });
                    }
                    KeyCode::Char('r') | KeyCode::Char('R') => {
                        app.repeat = !app.repeat;
                        app.status_message = format!("Repeat: {}", if app.repeat { "ON" } else { "OFF" });
                    }
                    KeyCode::Char('+') | KeyCode::Char('=') | KeyCode::Char(']') => {
                        app.audio_player.change_volume(0.05);
                    }
                    KeyCode::Char('-') | KeyCode::Char('_') | KeyCode::Char('[') => {
                        app.audio_player.change_volume(-0.05);
                    }
                    _ => {}
                }
            }
        }
    }

    app.audio_player.stop();
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    println!("Exited Fringe Spotify Player. Have a nice day! ♫");
    Ok(())
}
