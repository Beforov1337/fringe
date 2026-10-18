use ratatui::style::Color;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Config {
    // Theme: "monochrome", "synthwave", "matrix", "amber", "custom"
    pub theme: String,

    // CAVA Visualizer settings
    pub bar_width: usize,
    pub bar_gap: usize,
    pub framerate: u64,
    pub gravity: f64,
    pub sensitivity: f64,
    pub noise_reduction: f64,
    pub monstercat: f64,
    pub show_peaks: bool,
    // External custom command to run instead of built-in CAVA (e.g., "lavat -c cyan", "cava")
    pub visualizer_cmd: String,

    // Lyrics effects: "glow", "underline", "box", "dots", "bold"
    pub lyric_effect: String,

    // Custom colors (if theme is "custom" or for overriding)
    pub color_primary: Color,
    pub color_accent: Color,
    pub color_active_word: Color,
    pub color_sung_word: Color,
    pub color_inactive: Color,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            theme: "monochrome".to_string(),
            bar_width: 2,
            bar_gap: 1,
            framerate: 60,
            gravity: 28.0,
            sensitivity: 1.0,
            noise_reduction: 0.77,
            monstercat: 1.0,
            show_peaks: false,
            visualizer_cmd: String::new(),
            lyric_effect: "glow".to_string(),

            // Monochrome defaults (clean, elegant, minimal)
            color_primary: Color::Rgb(240, 240, 240),
            color_accent: Color::Rgb(255, 255, 255),
            color_active_word: Color::Rgb(255, 255, 255),
            color_sung_word: Color::Rgb(160, 160, 160),
            color_inactive: Color::Rgb(90, 90, 90),
        }
    }
}

impl Config {
    pub fn config_path() -> PathBuf {
        if let Ok(home) = std::env::var("HOME") {
            PathBuf::from(home).join(".config/fringe/fringe.conf")
        } else {
            PathBuf::from("fringe.conf")
        }
    }

    pub fn load_or_create_default() -> Self {
        let path = Self::config_path();
        if !path.exists() {
            if let Some(parent) = path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            let template = Self::default_config_template();
            let _ = fs::write(&path, template);
            return Self::default();
        }

        match fs::read_to_string(&path) {
            Ok(content) => Self::parse(&content),
            Err(_) => Self::default(),
        }
    }

    pub fn default_config_template() -> &'static str {
r#"# fringe configuration
# Location: ~/.config/fringe/fringe.conf
# Syntax: key = value

# Theme: monochrome, synthwave, matrix, amber, custom
theme = monochrome

# Visualizer (CAVA or custom program)
# To embed an external program inside the visualizer window (e.g. lavat, cava, pipes.sh):
# visualizer_cmd = lavat -c cyan -R 1
# Leave empty to use built-in CAVA visualizer
visualizer_cmd = 

bar_width = 2
bar_gap = 1
framerate = 60
sensitivity = 1.0
gravity = 28.0
noise_reduction = 0.77
monstercat = 1.0
show_peaks = false

# Lyrics active word effect: glow, underline, box, dots, bold
lyric_effect = glow

# Custom colors (hex #rrggbb or names: white, black, gray, cyan, green, yellow, red, blue, magenta)
# color_primary = #e0e0e0
# color_accent = #ffffff
# color_active_word = #ffffff
# color_sung_word = #909090
# color_inactive = #505050
"#
    }

    pub fn parse(content: &str) -> Self {
        let mut cfg = Self::default();

        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with(';') {
                continue;
            }

            if let Some((key, val)) = trimmed.split_once('=') {
                let k = key.trim().to_lowercase();
                let v = val.trim();

                match k.as_str() {
                    "theme" => {
                        cfg.theme = v.to_lowercase();
                        cfg.apply_theme(&cfg.theme.clone());
                    }
                    "bar_width" => {
                        if let Ok(w) = v.parse::<usize>() {
                            cfg.bar_width = w.clamp(1, 12);
                        }
                    }
                    "bar_gap" | "bar_spacing" => {
                        if let Ok(g) = v.parse::<usize>() {
                            cfg.bar_gap = g.clamp(0, 5);
                        }
                    }
                    "framerate" | "fps" => {
                        if let Ok(fps) = v.parse::<u64>() {
                            cfg.framerate = fps.clamp(15, 144);
                        }
                    }
                    "gravity" => {
                        if let Ok(g) = v.parse::<f64>() {
                            cfg.gravity = g.clamp(5.0, 100.0);
                        }
                    }
                    "sensitivity" => {
                        if let Ok(s) = v.parse::<f64>() {
                            cfg.sensitivity = s.clamp(0.1, 10.0);
                        }
                    }
                    "noise_reduction" => {
                        if let Ok(nr) = v.parse::<f64>() {
                            cfg.noise_reduction = nr.clamp(0.0, 1.0);
                        }
                    }
                    "monstercat" => {
                        if let Ok(m) = v.parse::<f64>() {
                            cfg.monstercat = m.clamp(0.0, 3.0);
                        } else if v.eq_ignore_ascii_case("true") || v == "1" {
                            cfg.monstercat = 1.0;
                        } else if v.eq_ignore_ascii_case("false") || v == "0" {
                            cfg.monstercat = 0.0;
                        }
                    }
                    "show_peaks" | "peaks" => {
                        cfg.show_peaks = v.eq_ignore_ascii_case("true") || v == "1";
                    }
                    "visualizer_cmd" | "visualizer" | "custom_visualizer" => {
                        cfg.visualizer_cmd = v.to_string();
                    }
                    "lyric_effect" => {
                        cfg.lyric_effect = v.to_lowercase();
                    }
                    "color_active_word" => {
                        if let Some(c) = parse_color(v) {
                            cfg.color_active_word = c;
                        }
                    }
                    "color_sung_word" => {
                        if let Some(c) = parse_color(v) {
                            cfg.color_sung_word = c;
                        }
                    }
                    "color_inactive" => {
                        if let Some(c) = parse_color(v) {
                            cfg.color_inactive = c;
                        }
                    }
                    "color_accent" => {
                        if let Some(c) = parse_color(v) {
                            cfg.color_accent = c;
                        }
                    }
                    "color_primary" => {
                        if let Some(c) = parse_color(v) {
                            cfg.color_primary = c;
                        }
                    }
                    _ => {}
                }
            }
        }

        cfg
    }

    pub fn apply_theme(&mut self, theme_name: &str) {
        match theme_name {
            "synthwave" => {
                self.color_primary = Color::Rgb(0, 240, 255);       // Cyan
                self.color_accent = Color::Rgb(255, 121, 198);      // Pink
                self.color_active_word = Color::Rgb(255, 235, 80);  // Gold
                self.color_sung_word = Color::Rgb(70, 240, 180);    // Mint
                self.color_inactive = Color::Rgb(90, 105, 130);     // Dim blue
            }
            "matrix" => {
                self.color_primary = Color::Rgb(50, 220, 90);
                self.color_accent = Color::Rgb(100, 255, 140);
                self.color_active_word = Color::Rgb(200, 255, 200);
                self.color_sung_word = Color::Rgb(40, 180, 80);
                self.color_inactive = Color::Rgb(25, 75, 40);
            }
            "amber" => {
                self.color_primary = Color::Rgb(255, 175, 0);
                self.color_accent = Color::Rgb(255, 215, 80);
                self.color_active_word = Color::Rgb(255, 245, 180);
                self.color_sung_word = Color::Rgb(200, 130, 20);
                self.color_inactive = Color::Rgb(100, 70, 20);
            }
            _ => {
                // Monochrome (Default)
                self.color_primary = Color::Rgb(235, 235, 235);
                self.color_accent = Color::Rgb(255, 255, 255);
                self.color_active_word = Color::Rgb(255, 255, 255);
                self.color_sung_word = Color::Rgb(155, 155, 155);
                self.color_inactive = Color::Rgb(80, 80, 80);
            }
        }
    }
}

fn parse_color(val: &str) -> Option<Color> {
    let clean = val.trim();
    if clean.starts_with('#') && (clean.len() == 7) {
        let r = u8::from_str_radix(&clean[1..3], 16).ok()?;
        let g = u8::from_str_radix(&clean[3..5], 16).ok()?;
        let b = u8::from_str_radix(&clean[5..7], 16).ok()?;
        return Some(Color::Rgb(r, g, b));
    }
    if clean.contains(',') {
        let parts: Vec<&str> = clean.split(',').collect();
        if parts.len() == 3 {
            let r: u8 = parts[0].trim().parse().ok()?;
            let g: u8 = parts[1].trim().parse().ok()?;
            let b: u8 = parts[2].trim().parse().ok()?;
            return Some(Color::Rgb(r, g, b));
        }
    }
    match clean.to_lowercase().as_str() {
        "white" => Some(Color::White),
        "black" => Some(Color::Black),
        "gray" | "grey" => Some(Color::Gray),
        "darkgray" | "darkgrey" => Some(Color::DarkGray),
        "cyan" => Some(Color::Cyan),
        "green" => Some(Color::Green),
        "yellow" => Some(Color::Yellow),
        "magenta" => Some(Color::Magenta),
        "red" => Some(Color::Red),
        "blue" => Some(Color::Blue),
        _ => None,
    }
}
