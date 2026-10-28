//! mpv JSON IPC client over a Windows named pipe.
//!
//! Two background connections are maintained:
//! - **Observer**: streams `time-pos`, `duration`, `pause`, `eof-reached`
//!   property-change events into a live [`MpvRuntime`] snapshot.
//! - **Command writer**: keeps ONE persistent connection open, drains a command
//!   queue, reads & discards each reply. mpv discards input from connections
//!   closed immediately after writing, so a persistent connection is required.
//!
//! `send()` merely enqueues, so UI-thread calls never block.

use std::collections::VecDeque;
use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};
use tracing::{debug, warn};

/// Freshness window: mpv emits `time-pos` property changes continuously while
/// playing; older snapshots mean the pipe or mpv is gone.
const STALE_AFTER: Duration = Duration::from_millis(2000);
const PIPE_CONNECT_RETRIES: usize = 50;
const PIPE_RETRY_DELAY: Duration = Duration::from_millis(100);
const CMD_CONNECT_RETRIES: usize = 10;

/// Real playback state reported by mpv via property-change events.
#[derive(Debug, Clone, Copy, Default)]
pub struct MpvRuntime {
    pub time_pos: f64,
    pub duration: f64,
    pub paused: bool,
    pub eof_reached: bool,
    pub connected: bool,
    pub last_update: Option<Instant>,
}

pub struct MpvIpcMonitor {
    pipe_name: String,
    runtime: Mutex<MpvRuntime>,
    queue: Mutex<VecDeque<Value>>,
    signal: Condvar,
    closing: AtomicBool,
}

impl MpvIpcMonitor {
    /// Starts the background property observer and command writer.
    pub fn start(pipe_name: String) -> Arc<Self> {
        let monitor = Arc::new(Self {
            pipe_name,
            runtime: Mutex::new(MpvRuntime::default()),
            queue: Mutex::new(VecDeque::new()),
            signal: Condvar::new(),
            closing: AtomicBool::new(false),
        });
        let observer = monitor.clone();
        let _ = std::thread::Builder::new()
            .name("wintube-mpv-obs".into())
            .spawn(move || observer.observer_loop());
        let writer = monitor.clone();
        let _ = std::thread::Builder::new()
            .name("wintube-mpv-cmd".into())
            .spawn(move || writer.writer_loop());
        monitor
    }

    pub fn pipe_name(&self) -> &str {
        &self.pipe_name
    }

    fn connect_pipe(&self, retries: usize) -> Option<File> {
        for _ in 0..retries {
            if self.closing.load(Ordering::SeqCst) {
                return None;
            }
            if let Ok(file) = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&self.pipe_name)
            {
                return Some(file);
            }
            std::thread::sleep(PIPE_RETRY_DELAY);
        }
        None
    }

    fn observer_loop(self: Arc<Self>) {
        let mut file = match self.connect_pipe(PIPE_CONNECT_RETRIES) {
            Some(file) => file,
            None => {
                warn!("mpv IPC pipe never appeared: {}", self.pipe_name);
                return;
            }
        };

        // Register property observers (events stream for the connection lifetime).
        // IDs must be unique: mpv replaces an existing observation when the same
        // id is registered again, which would silently drop all but the last.
        for (id, prop) in ["time-pos", "duration", "pause", "eof-reached"]
            .into_iter()
            .enumerate()
        {
            let cmd = json!({ "command": ["observe_property", id + 1, prop] });
            if file.write_all(format!("{cmd}\n").as_bytes()).is_err() {
                break;
            }
        }
        let _ = file.flush();

        let reader = BufReader::new(file);
        for line in reader.lines() {
            if self.closing.load(Ordering::SeqCst) {
                break;
            }
            let Ok(line) = line else { break };
            let Ok(value) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if value.get("event").and_then(|e| e.as_str()) != Some("property-change") {
                continue;
            }
            let name = value.get("name").and_then(|n| n.as_str()).unwrap_or("");
            let data = value.get("data").cloned().unwrap_or(Value::Null);
            let Ok(mut runtime) = self.runtime.lock() else {
                break;
            };
            runtime.connected = true;
            runtime.last_update = Some(Instant::now());
            match name {
                "time-pos" => {
                    if let Some(pos) = data.as_f64() {
                        runtime.time_pos = pos;
                    }
                }
                "duration" => {
                    if let Some(dur) = data.as_f64() {
                        runtime.duration = dur;
                    }
                }
                "pause" => {
                    if let Some(paused) = data.as_bool() {
                        runtime.paused = paused;
                    }
                }
                "eof-reached" => {
                    if let Some(eof) = data.as_bool() {
                        runtime.eof_reached = eof;
                    }
                }
                _ => {}
            }
        }

        if let Ok(mut runtime) = self.runtime.lock() {
            runtime.connected = false;
        }
        debug!("mpv IPC observer terminated for {}", self.pipe_name);
    }

    fn writer_loop(self: Arc<Self>) {
        // write_half sends commands; read_half drains replies & broadcast events
        // through a PERSISTENT BufReader (a throwaway reader would swallow
        // buffered lines on drop).
        let mut write_half: Option<File> = None;
        let mut read_half: Option<BufReader<File>> = None;

        loop {
            if self.closing.load(Ordering::SeqCst) {
                return;
            }

            // Fetch the next queued command (bounded wait so shutdown is honored).
            let command = {
                let mut queue = self.queue.lock().unwrap_or_else(|p| p.into_inner());
                loop {
                    if let Some(cmd) = queue.pop_front() {
                        break Some(cmd);
                    }
                    if self.closing.load(Ordering::SeqCst) {
                        break None;
                    }
                    let (guard, _timeout) = self
                        .signal
                        .wait_timeout(queue, Duration::from_millis(500))
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    queue = guard;
                }
            };
            let Some(command) = command else { return };

            // Maintain one persistent connection (mpv needs it kept open).
            if write_half.is_none() {
                match self.connect_pipe(CMD_CONNECT_RETRIES) {
                    Some(file) => match file.try_clone() {
                        Ok(write_clone) => {
                            write_half = Some(write_clone);
                            read_half = Some(BufReader::new(file));
                        }
                        Err(_) => continue,
                    },
                    None => {
                        warn!("mpv IPC command pipe unavailable: {}", self.pipe_name);
                        continue;
                    }
                }
            }

            let connected = write_half
                .as_mut()
                .map(|f| {
                    f.write_all(format!("{command}\n").as_bytes()).is_ok() && f.flush().is_ok()
                })
                .unwrap_or(false);
            if !connected {
                // Connection lost: requeue, drop both halves, reconnect next time.
                self.queue
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .push_front(command);
                write_half = None;
                read_half = None;
                continue;
            }

            // Read lines until the command's own reply (contains "error") arrives,
            // discarding interleaved broadcast events. EOF means mpv exited.
            if let Some(reader) = read_half.as_mut() {
                loop {
                    let mut line = String::new();
                    match reader.read_line(&mut line) {
                        Ok(0) => {
                            write_half = None;
                            read_half = None;
                            break;
                        }
                        Ok(_) => {
                            let trimmed = line.trim();
                            if trimmed.contains("\"error\"") {
                                if !trimmed.contains("\"error\":\"success\"") {
                                    warn!("mpv IPC command {command} -> {trimmed}");
                                }
                                break;
                            }
                            // Broadcast event (e.g. audio-reconfig) — keep draining.
                        }
                        Err(_) => {
                            write_half = None;
                            read_half = None;
                            break;
                        }
                    }
                }
            }
        }
    }

    /// Live snapshot of mpv's playback state.
    pub fn snapshot(&self) -> MpvRuntime {
        *self
            .runtime
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// True when mpv is connected and reporting fresh property updates.
    pub fn is_fresh(&self) -> bool {
        let runtime = self
            .runtime
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        runtime.connected
            && runtime
                .last_update
                .map(|t| t.elapsed() < STALE_AFTER)
                .unwrap_or(false)
    }

    /// Enqueues an mpv command for delivery on the persistent writer
    /// connection. Non-blocking.
    pub fn send(&self, command: &Value) -> bool {
        if self.closing.load(Ordering::SeqCst) {
            return false;
        }
        self.queue
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push_back(command.clone());
        self.signal.notify_one();
        true
    }

    /// Signals both background threads to wind down.
    pub fn shutdown(&self) {
        self.closing.store(true, Ordering::SeqCst);
        self.signal.notify_all();
    }
}

/// Unique per-process IPC pipe name so multiple WinTube instances don't collide.
pub fn new_pipe_name() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!(r"\\.\pipe\wintube-mpv-{}-{}", std::process::id(), millis)
}
