use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use std::io::Read;
use std::sync::{Arc, Mutex};
use std::thread;

pub struct ExternalVisualizer {
    parser: Arc<Mutex<vt100::Parser>>,
    master: Option<Box<dyn MasterPty + Send>>,
    _child: Option<Box<dyn portable_pty::Child + Send + Sync>>,
    last_size: (u16, u16),
    command_str: String,
}

impl ExternalVisualizer {
    pub fn new(command_str: String) -> Self {
        Self {
            parser: Arc::new(Mutex::new(vt100::Parser::new(24, 80, 0))),
            master: None,
            _child: None,
            last_size: (0, 0),
            command_str,
        }
    }

    pub fn resize_if_needed(&mut self, width: u16, height: u16) {
        if (width, height) == self.last_size || width < 2 || height < 2 {
            return;
        }
        self.last_size = (width, height);

        if let Ok(mut parser) = self.parser.lock() {
            parser.screen_mut().set_size(height, width);
        }

        if let Some(ref master) = self.master {
            let _ = master.resize(PtySize {
                rows: height,
                cols: width,
                pixel_width: 0,
                pixel_height: 0,
            });
        } else {
            self.spawn_process(width, height);
        }
    }

    fn spawn_process(&mut self, width: u16, height: u16) {
        if self.command_str.trim().is_empty() {
            return;
        }

        let pty_system = native_pty_system();
        let pair = match pty_system.openpty(PtySize {
            rows: height,
            cols: width,
            pixel_width: 0,
            pixel_height: 0,
        }) {
            Ok(p) => p,
            Err(_) => return,
        };

        let mut cmd = CommandBuilder::new("sh");
        cmd.arg("-c");
        cmd.arg(&self.command_str);
        cmd.env("TERM", "xterm-256color");

        let child = match pair.slave.spawn_command(cmd) {
            Ok(c) => c,
            Err(_) => return,
        };

        let mut reader = match pair.master.try_clone_reader() {
            Ok(r) => r,
            Err(_) => return,
        };

        self.master = Some(pair.master);
        self._child = Some(child);

        let parser = Arc::clone(&self.parser);
        thread::spawn(move || {
            let mut buf = [0u8; 4096];
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 {
                    break;
                }
                if let Ok(mut p) = parser.lock() {
                    p.process(&buf[..n]);
                }
            }
        });
    }

    pub fn render(&mut self, area: Rect, buf: &mut Buffer) {
        if area.width < 2 || area.height < 2 {
            return;
        }

        self.resize_if_needed(area.width, area.height);

        let parser = match self.parser.lock() {
            Ok(p) => p,
            Err(_) => return,
        };

        let screen = parser.screen();
        let (rows, cols) = screen.size();

        for r in 0..(area.height.min(rows)) {
            let target_y = area.y + r;
            for c in 0..(area.width.min(cols)) {
                let target_x = area.x + c;

                if let Some(cell) = screen.cell(r, c) {
                    let ch = cell.contents().chars().next().unwrap_or(' ');
                    let mut style = Style::default();

                    let fg = cell.fgcolor();
                    let bg = cell.bgcolor();

                    if let Some(color) = Self::convert_color(fg) {
                        style = style.fg(color);
                    }
                    if let Some(color) = Self::convert_color(bg) {
                        style = style.bg(color);
                    }
                    if cell.bold() {
                        style = style.add_modifier(Modifier::BOLD);
                    }
                    if cell.italic() {
                        style = style.add_modifier(Modifier::ITALIC);
                    }
                    if cell.underline() {
                        style = style.add_modifier(Modifier::UNDERLINED);
                    }

                    let cell_ref = &mut buf[(target_x, target_y)];
                    cell_ref.set_char(ch);
                    cell_ref.set_style(style);
                }
            }
        }
    }

    fn convert_color(color: vt100::Color) -> Option<Color> {
        match color {
            vt100::Color::Default => None,
            vt100::Color::Idx(idx) => Some(Color::Indexed(idx)),
            vt100::Color::Rgb(r, g, b) => Some(Color::Rgb(r, g, b)),
        }
    }
}
