use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use std::f64::consts::PI;

use crate::audio::VisualizerBuffer;
use crate::config::Config;

pub struct CavaVisualizer {
    time: f64,
    bar_heights: Vec<f64>,
    cava_fall: Vec<f64>,
    cava_mem: Vec<f64>,
    cava_peak: Vec<f64>,
    peak_caps: Vec<f64>,
}

impl CavaVisualizer {
    pub fn new() -> Self {
        Self {
            time: 0.0,
            bar_heights: Vec::new(),
            cava_fall: Vec::new(),
            cava_mem: Vec::new(),
            cava_peak: Vec::new(),
            peak_caps: Vec::new(),
        }
    }

    pub fn update(&mut self, dt: f64, cfg: &Config) {
        self.time += dt;
        if self.time > 100000.0 {
            self.time = 0.0;
        }

        // Peak caps gravity falloff (if enabled)
        let peak_gravity = (cfg.gravity * 0.45) * dt;
        for p in self.peak_caps.iter_mut() {
            *p = (*p - peak_gravity).max(0.0);
        }
    }

    /// Render authentic CAVA spectrum equalizer into ratatui Buffer
    pub fn render(&mut self, area: Rect, buf: &mut Buffer, audio_vis: &VisualizerBuffer, cfg: &Config) {
        if area.width < 4 || area.height < 2 {
            return;
        }

        let width = area.width as usize;
        let height = area.height as usize;

        // Clean, readable CAVA bars: width and spacing from config
        let b_width = cfg.bar_width.clamp(1, 12);
        let b_gap = cfg.bar_gap.clamp(0, 5);
        let stride = b_width + b_gap;

        let num_bars = (width / stride).max(1);

        if self.bar_heights.len() != num_bars {
            self.bar_heights = vec![0.0; num_bars];
            self.cava_fall = vec![0.0; num_bars];
            self.cava_mem = vec![0.0; num_bars];
            self.cava_peak = vec![0.0; num_bars];
            self.peak_caps = vec![0.0; num_bars];
        }

        let is_active = audio_vis.is_active.load(std::sync::atomic::Ordering::Relaxed);
        let _energy = (audio_vis.get_energy() as f64).clamp(0.0, 1.0);
        let samples = audio_vis.get_samples_snapshot(512);

        // Sub-height blocks: 8 levels per character cell (authentic CAVA block characters)
        const BLOCKS: [char; 8] = [' ', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
        let max_sub_height = (height as f64) * 8.0;

        // 1. Calculate raw target heights using logarithmic frequency bands (CAVA standard)
        let sample_len = samples.len().max(1);
        let mut raw_targets = vec![0.0; num_bars];

        for i in 0..num_bars {
            if is_active && !samples.is_empty() {
                // Logarithmic frequency distribution
                let bin_ratio = (i as f64) / (num_bars as f64);
                let start_idx = ((bin_ratio.powf(1.8)) * (sample_len as f64 * 0.75)) as usize;
                let chunk_size = (sample_len / num_bars).clamp(2, 16);
                let end_idx = (start_idx + chunk_size).min(sample_len);

                let slice = &samples[start_idx..end_idx];
                let bin_rms = if !slice.is_empty() {
                    let sum: f64 = slice.iter().map(|&s| (s as f64) * (s as f64)).sum();
                    (sum / slice.len() as f64).sqrt()
                } else {
                    0.0
                };

                // Authentic CAVA frequency weighting
                let eq_weight = 0.55 + (1.0 - bin_ratio) * 0.35;
                let val = (bin_rms * 1.8 * eq_weight * cfg.sensitivity).clamp(0.0, 1.0);
                let shaped = val.powf(1.6);

                raw_targets[i] = (shaped * 0.85 * max_sub_height).clamp(0.0, max_sub_height - 1.0);
            } else {
                // Idle ambient breathing spectrum
                let wave1 = (self.time * 2.0 + (i as f64) * 0.22).sin() * 0.5 + 0.5;
                let wave2 = (self.time * 1.2 - (i as f64) * 0.15 + PI * 0.5).cos() * 0.5 + 0.5;
                let idle = (wave1 * 0.55 + wave2 * 0.45) * (max_sub_height * 0.18);
                raw_targets[i] = idle.clamp(0.0, max_sub_height * 0.25);
            }
        }

        // 2. Monstercat-style wave smoothing across adjacent bars (authentic cava.c monstercat_filter)
        if cfg.monstercat > 0.05 {
            let m_factor = cfg.monstercat * 1.5;
            for z in 0..num_bars {
                for m_y in (0..z).rev() {
                    let de = (z - m_y) as i32;
                    let smoothed = raw_targets[z] / m_factor.powi(de);
                    if smoothed > raw_targets[m_y] {
                        raw_targets[m_y] = smoothed;
                    }
                }
                for m_y in (z + 1)..num_bars {
                    let de = (m_y - z) as i32;
                    let smoothed = raw_targets[z] / m_factor.powi(de);
                    if smoothed > raw_targets[m_y] {
                        raw_targets[m_y] = smoothed;
                    }
                }
            }
        }

        // 3. CAVA physics smoothing: gravity falloff and noise reduction integral
        // Derived from cavacore.c:
        // double gravity_mod = pow((66 / framerate), 2.5) * 2 / noise_reduction;
        let fps = (cfg.framerate as f64).clamp(15.0, 144.0);
        let nr = cfg.noise_reduction.clamp(0.1, 1.0);
        let framerate_mod = 66.0 / fps;
        let gravity_mod = framerate_mod.powf(2.0) * (cfg.gravity / 28.0) * 0.035 / nr;
        let integral_mod = framerate_mod.powf(0.1);

        for i in 0..num_bars {
            let target = raw_targets[i];

            if target < self.bar_heights[i] {
                // Bar is falling according to gravity
                let fall = self.cava_fall[i];
                let new_h = self.cava_peak[i] * (1.0 - fall * fall * gravity_mod);
                self.bar_heights[i] = new_h.max(0.0);
                self.cava_fall[i] += 0.035;
            } else {
                // Bar is rising instantly on beat (fast attack, no delay)
                self.cava_peak[i] = target;
                self.cava_fall[i] = 0.0;
                self.bar_heights[i] = target;
            }

            // Light integral memory smoothing
            let integrated = self.cava_mem[i] * (nr * 0.15) / integral_mod + self.bar_heights[i] * (1.0 - nr * 0.15);
            self.cava_mem[i] = integrated;
            self.bar_heights[i] = integrated.clamp(0.0, max_sub_height - 1.0);

            // Peak cap tracking
            if self.bar_heights[i] > self.peak_caps[i] {
                self.peak_caps[i] = self.bar_heights[i];
            }
        }

        // 4. Color Palette setup based on theme
        let is_mono = cfg.theme == "monochrome" || cfg.theme == "mono";

        // Render bars into Ratatui buffer
        for i in 0..num_bars {
            let start_col = area.x + (i * stride) as u16;
            if start_col >= area.x + area.width {
                break;
            }

            let sub_h = self.bar_heights[i];
            let full_cells = (sub_h / 8.0).floor() as usize;
            let partial = ((sub_h % 8.0).floor() as usize).min(7);

            let peak_sub = self.peak_caps[i];
            let peak_cell_y = (peak_sub / 8.0).floor() as usize;

            for r in 0..height {
                let from_bottom = height - 1 - r;
                let cell_y = area.y + (r as u16);
                let vert_ratio = (from_bottom as f64) / (height as f64);

                let color = if is_mono {
                    // Clean CAVA Monochrome Gradient: Soft gray base -> silver -> bright white top
                    if vert_ratio < 0.35 {
                        Color::Rgb(140, 140, 140)
                    } else if vert_ratio < 0.70 {
                        Color::Rgb(200, 200, 200)
                    } else {
                        Color::Rgb(255, 255, 255)
                    }
                } else if cfg.theme == "matrix" {
                    if vert_ratio < 0.4 {
                        Color::Rgb(40, 180, 70)
                    } else if vert_ratio < 0.75 {
                        Color::Rgb(80, 230, 120)
                    } else {
                        Color::Rgb(200, 255, 210)
                    }
                } else if cfg.theme == "amber" {
                    if vert_ratio < 0.4 {
                        Color::Rgb(200, 120, 20)
                    } else if vert_ratio < 0.75 {
                        Color::Rgb(255, 175, 40)
                    } else {
                        Color::Rgb(255, 235, 150)
                    }
                } else {
                    // Synthwave / Custom
                    if vert_ratio < 0.4 {
                        cfg.color_primary
                    } else if vert_ratio < 0.75 {
                        Color::Rgb(189, 147, 249)
                    } else {
                        cfg.color_accent
                    }
                };

                // Draw solid bar columns of b_width characters
                for w in 0..b_width {
                    let col = start_col + (w as u16);
                    if col >= area.x + area.width {
                        break;
                    }

                    let cell = &mut buf[(col, cell_y)];

                    if from_bottom < full_cells {
                        cell.set_char('█');
                        cell.set_style(Style::default().fg(color));
                    } else if from_bottom == full_cells && partial > 0 {
                        cell.set_char(BLOCKS[partial]);
                        cell.set_style(Style::default().fg(color));
                    } else if cfg.show_peaks && from_bottom == peak_cell_y && peak_sub > 2.0 {
                        cell.set_char('▀');
                        cell.set_style(Style::default().fg(Color::Rgb(255, 255, 255)));
                    } else {
                        cell.set_char(' ');
                    }
                }

                // Draw spacing between bars (b_gap characters)
                for g in 0..b_gap {
                    let col = start_col + (b_width as u16) + (g as u16);
                    if col < area.x + area.width {
                        let cell = &mut buf[(col, cell_y)];
                        cell.set_char(' ');
                    }
                }
            }
        }
    }
}
