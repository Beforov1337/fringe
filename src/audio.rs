use anyhow::{anyhow, Result};
use rodio::buffer::SamplesBuffer;
use rodio::{DeviceSinkBuilder, MixerDeviceSink, Player};
use std::collections::VecDeque;
use std::env;
use std::fs::{self, File};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::{Read, Write};
use std::num::NonZero;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

#[derive(Clone)]
pub struct VisualizerBuffer {
    pub samples: Arc<Mutex<VecDeque<f32>>>,
    pub current_energy: Arc<Mutex<f32>>,
    pub is_active: Arc<AtomicBool>,
}

impl VisualizerBuffer {
    pub fn new() -> Self {
        Self {
            samples: Arc::new(Mutex::new(VecDeque::with_capacity(1024))),
            current_energy: Arc::new(Mutex::new(0.0)),
            is_active: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn push_samples(&self, new_samples: &[f32]) {
        if let Ok(mut buf) = self.samples.lock() {
            for &s in new_samples {
                if buf.len() >= 512 {
                    buf.pop_front();
                }
                buf.push_back(s);
            }
        }

        let sum_sq: f32 = new_samples.iter().map(|&s| s * s).sum();
        let rms = (sum_sq / (new_samples.len().max(1) as f32)).sqrt();
        if let Ok(mut energy) = self.current_energy.lock() {
            *energy = *energy * 0.7 + rms * 0.3;
        }
    }

    pub fn get_samples_snapshot(&self, count: usize) -> Vec<f32> {
        if let Ok(buf) = self.samples.lock() {
            if buf.is_empty() {
                return vec![0.0; count];
            }
            buf.iter().copied().take(count).collect()
        } else {
            vec![0.0; count]
        }
    }

    pub fn get_energy(&self) -> f32 {
        if let Ok(energy) = self.current_energy.lock() {
            *energy
        } else {
            0.0
        }
    }
}

pub struct AudioPlayer {
    _device_sink: MixerDeviceSink,
    player: Arc<Mutex<Player>>,
    visualizer_buffer: VisualizerBuffer,
    active_children: Arc<Mutex<Option<(Option<Child>, Option<Child>)>>>,
    pub is_playing: Arc<AtomicBool>,
    pub generation: Arc<std::sync::atomic::AtomicU64>,
    pub volume: Arc<Mutex<f32>>,
    pub elapsed_secs: Arc<Mutex<f64>>,
    pub total_secs: Arc<Mutex<f64>>,
}

impl AudioPlayer {
    pub fn new() -> Result<Self> {
        let mut sink_builder = DeviceSinkBuilder::open_default_sink()
            .map_err(|e| anyhow!("Failed to open default audio output sink: {}", e))?;
        sink_builder.log_on_drop(false);

        let player = Player::connect_new(&sink_builder.mixer());

        let vis_buf = VisualizerBuffer::new();
        let active_children = Arc::new(Mutex::new(None));
        let is_playing = Arc::new(AtomicBool::new(false));
        let generation = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let volume = Arc::new(Mutex::new(0.8));
        let elapsed_secs = Arc::new(Mutex::new(0.0));
        let total_secs = Arc::new(Mutex::new(0.0));

        player.set_volume(0.8);

        let cache_dir = Self::cache_dir();
        let _ = fs::create_dir_all(cache_dir);

        Ok(Self {
            _device_sink: sink_builder,
            player: Arc::new(Mutex::new(player)),
            visualizer_buffer: vis_buf,
            active_children,
            is_playing,
            generation,
            volume,
            elapsed_secs,
            total_secs,
        })
    }

    pub fn visualizer(&self) -> VisualizerBuffer {
        self.visualizer_buffer.clone()
    }

    pub fn cache_dir() -> PathBuf {
        if let Ok(home) = env::var("HOME") {
            PathBuf::from(home).join(".cache/fringe/audio")
        } else {
            PathBuf::from("/tmp/fringe_cache")
        }
    }

    pub fn get_cache_path(query: &str) -> PathBuf {
        let clean = Self::sanitize_query(query);
        let mut hasher = DefaultHasher::new();
        clean.hash(&mut hasher);
        let hash = hasher.finish();

        let safe_name: String = clean
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
            .take(30)
            .collect();

        Self::cache_dir().join(format!("{}_{:x}.pcm", safe_name, hash))
    }

    fn find_yt_dlp() -> String {
        if let Ok(home) = env::var("HOME") {
            let local_bin = PathBuf::from(home).join(".local/bin/yt-dlp");
            if local_bin.exists() {
                return local_bin.to_string_lossy().to_string();
            }
        }
        "yt-dlp".to_string()
    }

    pub fn sanitize_query(query: &str) -> String {
        query
            .replace('\u{a0}', " ")
            .replace('"', "")
            .trim()
            .to_string()
    }

    pub fn prebuffer_track(query: String) {
        let clean = Self::sanitize_query(&query);
        let cache_path = Self::get_cache_path(&clean);

        if cache_path.exists() {
            if let Ok(meta) = fs::metadata(&cache_path) {
                if meta.len() > 200_000 {
                    return;
                }
            }
        }

        let part_path = cache_path.with_extension("pcm.part");
        let yt_dlp_bin = Self::find_yt_dlp();
        let search_arg = format!("ytsearch1:{}", clean);

        thread::spawn(move || {
            let mut p1 = Command::new(yt_dlp_bin);
            p1.args(&[
                "-f", "ba/b",
                "--no-playlist",
                "--no-warnings",
                "--no-check-certificates",
                "--buffer-size", "16k",
                "-o", "-",
                &search_arg,
            ]);
            p1.stdout(Stdio::piped());
            p1.stderr(Stdio::null());

            let Ok(mut child1) = p1.spawn() else { return };
            let Some(stdout1) = child1.stdout.take() else { return };

            let mut p2 = Command::new("ffmpeg");
            p2.args(&[
                "-loglevel", "error",
                "-i", "pipe:0",
                "-f", "s16le",
                "-ar", "44100",
                "-ac", "2",
                "pipe:1",
            ]);
            p2.stdin(stdout1);
            p2.stdout(Stdio::piped());
            p2.stderr(Stdio::null());

            let Ok(mut child2) = p2.spawn() else {
                let _ = child1.kill();
                return;
            };

            let Some(mut stdout2) = child2.stdout.take() else {
                let _ = child2.kill();
                let _ = child1.kill();
                return;
            };

            let Ok(mut file) = File::create(&part_path) else { return };
            let mut buf = vec![0u8; 16384];

            while let Ok(n) = stdout2.read(&mut buf) {
                if n == 0 { break; }
                if file.write_all(&buf[..n]).is_err() { break; }
            }

            let _ = child2.wait();
            let _ = child1.wait();

            if let Ok(meta) = fs::metadata(&part_path) {
                if meta.len() > 200_000 {
                    let _ = fs::rename(part_path, cache_path);
                }
            }
        });
    }

    pub fn play_search_query<F>(&self, raw_query: String, duration_secs: f64, on_first_audio: F) -> Result<()>
    where
        F: FnOnce() + Send + 'static,
    {
        self.stop();

        if let Ok(mut el) = self.elapsed_secs.lock() {
            *el = 0.0;
        }
        if let Ok(mut tot) = self.total_secs.lock() {
            *tot = duration_secs;
        }

        let clean_query = Self::sanitize_query(&raw_query);
        let cache_path = Self::get_cache_path(&clean_query);

        // Fast path: play from local disk cache
        if cache_path.exists() {
            if let Ok(meta) = fs::metadata(&cache_path) {
                if meta.len() > 200_000 {
                    if let Ok(file) = File::open(&cache_path) {
                        return self.spawn_audio_feeder(Box::new(file), None, None, Box::new(on_first_audio));
                    }
                }
            }
        }

        // Network streaming path
        let yt_dlp_bin = Self::find_yt_dlp();
        let search_arg = format!("ytsearch1:{}", clean_query);

        let mut p1 = Command::new(yt_dlp_bin);
        p1.args(&[
            "-f", "ba/b",
            "--no-playlist",
            "--no-warnings",
            "--no-check-certificates",
            "--buffer-size", "16k",
            "-o", "-",
            &search_arg,
        ]);
        p1.stdout(Stdio::piped());
        p1.stderr(Stdio::null());

        let mut child1 = p1.spawn().map_err(|e| anyhow!("Failed to spawn yt-dlp: {}", e))?;
        let stdout1 = child1.stdout.take().ok_or_else(|| anyhow!("yt-dlp stdout missing"))?;

        let mut p2 = Command::new("ffmpeg");
        p2.args(&[
            "-loglevel", "error",
            "-i", "pipe:0",
            "-f", "s16le",
            "-ar", "44100",
            "-ac", "2",
            "pipe:1",
        ]);
        p2.stdin(stdout1);
        p2.stdout(Stdio::piped());
        p2.stderr(Stdio::null());

        let mut child2 = p2.spawn().map_err(|e| anyhow!("Failed to spawn ffmpeg: {}", e))?;
        let stdout2 = child2.stdout.take().ok_or_else(|| anyhow!("ffmpeg stdout missing"))?;

        let part_path = cache_path.with_extension("pcm.part");
        let cache_file = File::create(&part_path).ok();

        self.spawn_audio_feeder(
            Box::new(TeeReader {
                reader: stdout2,
                writer: cache_file,
                part_path: Some(part_path),
                final_path: Some(cache_path),
            }),
            Some(child1),
            Some(child2),
            Box::new(on_first_audio),
        )
    }

    fn spawn_audio_feeder(
        &self,
        mut reader: Box<dyn Read + Send>,
        child1: Option<Child>,
        child2: Option<Child>,
        on_first_audio: Box<dyn FnOnce() + Send>,
    ) -> Result<()> {
        if let Ok(mut current) = self.active_children.lock() {
            *current = Some((child1, child2));
        }

        let current_gen = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let gen_clone = Arc::clone(&self.generation);
        let player = Arc::clone(&self.player);
        let vis_buf = self.visualizer_buffer.clone();
        let is_playing = Arc::clone(&self.is_playing);
        let elapsed_secs = Arc::clone(&self.elapsed_secs);

        is_playing.store(true, Ordering::SeqCst);
        vis_buf.is_active.store(true, Ordering::SeqCst);

        thread::spawn(move || {
            let mut byte_buf = vec![0u8; 4096];
            let mut sample_buf = Vec::with_capacity(2048);
            let mut first_trigger: Option<Box<dyn FnOnce() + Send>> = Some(on_first_audio);

            let channels = NonZero::new(2).unwrap();
            let sample_rate = NonZero::new(44100).unwrap();

            while is_playing.load(Ordering::SeqCst) && gen_clone.load(Ordering::SeqCst) == current_gen {
                let (is_paused, queue_full, exact_pos) = if let Ok(p) = player.lock() {
                    (p.is_paused(), p.len() >= 3, p.get_pos().as_secs_f64())
                } else {
                    (false, false, 0.0)
                };

                // Sync elapsed_secs with exact physical hardware output position
                if exact_pos > 0.0 && gen_clone.load(Ordering::SeqCst) == current_gen {
                    if let Ok(mut el) = elapsed_secs.lock() {
                        *el = exact_pos;
                    }
                }

                if is_paused {
                    thread::sleep(Duration::from_millis(30));
                    continue;
                }

                if queue_full {
                    thread::sleep(Duration::from_millis(8));
                    continue;
                }

                match reader.read(&mut byte_buf) {
                    Ok(0) => {
                        break;
                    }
                    Ok(n) => {
                        sample_buf.clear();
                        let num_samples = n / 2;
                        for i in 0..num_samples {
                            let sample_bytes = [byte_buf[i * 2], byte_buf[i * 2 + 1]];
                            let sample_i16 = i16::from_le_bytes(sample_bytes);
                            let sample_f32 = (sample_i16 as f32) / 32768.0;
                            sample_buf.push(sample_f32);
                        }

                        vis_buf.push_samples(&sample_buf);

                        let source = SamplesBuffer::new(channels, sample_rate, sample_buf.clone());
                        if let Ok(p) = player.lock() {
                            p.append(source);
                        }

                        if let Some(cb) = first_trigger.take() {
                            cb();
                        }
                    }
                    Err(_) => {
                        break;
                    }
                }
            }

            vis_buf.is_active.store(false, Ordering::SeqCst);
        });

        Ok(())
    }

    pub fn toggle_pause(&self) {
        if let Ok(player) = self.player.lock() {
            if player.is_paused() {
                player.play();
                self.visualizer_buffer.is_active.store(true, Ordering::SeqCst);
            } else {
                player.pause();
                self.visualizer_buffer.is_active.store(false, Ordering::SeqCst);
            }
        }
    }

    pub fn is_paused(&self) -> bool {
        if let Ok(player) = self.player.lock() {
            player.is_paused()
        } else {
            false
        }
    }

    pub fn set_volume(&self, vol: f32) {
        let clamped = vol.clamp(0.0, 1.5);
        if let Ok(mut v) = self.volume.lock() {
            *v = clamped;
        }
        if let Ok(player) = self.player.lock() {
            player.set_volume(clamped);
        }
    }

    pub fn change_volume(&self, delta: f32) {
        let current = self.get_volume();
        self.set_volume(current + delta);
    }

    pub fn get_volume(&self) -> f32 {
        if let Ok(v) = self.volume.lock() {
            *v
        } else {
            0.8
        }
    }

    pub fn stop(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.is_playing.store(false, Ordering::SeqCst);
        self.visualizer_buffer.is_active.store(false, Ordering::SeqCst);

        // Clear visualizer samples buffer
        if let Ok(mut buf) = self.visualizer_buffer.samples.lock() {
            buf.clear();
        }

        if let Ok(mut current) = self.active_children.lock() {
            if let Some((child1, child2)) = current.take() {
                if let Some(mut c2) = child2 {
                    let _ = c2.kill();
                    let _ = c2.wait();
                }
                if let Some(mut c1) = child1 {
                    let _ = c1.kill();
                    let _ = c1.wait();
                }
            }
        }

        if let Ok(mut player_guard) = self.player.lock() {
            // Stop and empty current player
            player_guard.stop();
            let fresh_player = Player::connect_new(&self._device_sink.mixer());
            fresh_player.set_volume(self.get_volume());
            *player_guard = fresh_player;
        }
    }
}

struct TeeReader {
    reader: std::process::ChildStdout,
    writer: Option<File>,
    part_path: Option<PathBuf>,
    final_path: Option<PathBuf>,
}

impl Read for TeeReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.reader.read(buf)?;
        if n > 0 {
            if let Some(ref mut w) = self.writer {
                let _ = w.write_all(&buf[..n]);
            }
        } else {
            if let Some(w) = self.writer.take() {
                drop(w);
                if let (Some(part), Some(fin)) = (&self.part_path, &self.final_path) {
                    if let Ok(meta) = fs::metadata(part) {
                        if meta.len() > 200_000 {
                            let _ = fs::rename(part, fin);
                        }
                    }
                }
            }
        }
        Ok(n)
    }
}

impl Drop for AudioPlayer {
    fn drop(&mut self) {
        self.stop();
    }
}
