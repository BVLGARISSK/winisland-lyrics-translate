//! Runs the bundled NLLB worker and translates a whole song at once.

use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, VecDeque};
use std::hash::{Hash, Hasher};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::Duration;

use winisland_plugin_api::sdk::Host;

const BATCH: usize = 8;
const MAX_LINES: usize = 300;
const MAX_BATCH_BYTES: usize = 24_000;
const MAX_REPLY_BYTES: u64 = 512 * 1024;
const MAX_LOGGED_STDERR_LINES: usize = 64;
const REPLY_TIMEOUT: Duration = Duration::from_secs(120);
const CACHED_SONGS: usize = 8;
const LOG_WARN: u32 = 2;

pub(crate) struct Translator {
    host: Host,
    worker_dir: PathBuf,
    model_dir: PathBuf,
    worker: Option<Worker>,
    songs: VecDeque<(u64, Vec<Option<String>>)>,
}

impl Translator {
    pub(crate) fn new(host: Host, worker_dir: PathBuf, model_dir: PathBuf) -> Self {
        Self {
            host,
            worker_dir,
            model_dir,
            worker: None,
            songs: VecDeque::new(),
        }
    }

    pub(crate) fn model_ready(&self) -> bool {
        self.model_dir.join("model.bin").is_file()
    }

    pub(crate) fn model_dir(&self) -> &Path {
        &self.model_dir
    }

    /// Stops the worker process; a later request starts a new one.
    pub(crate) fn stop(&mut self) {
        self.worker = None;
    }

    /// Returns the translation of one line, translating the whole song on its first request.
    pub(crate) fn line(&mut self, song: &str, index: usize, source: &str, target: &str) -> String {
        let key = song_key(song, source, target);
        if !self.songs.iter().any(|(cached, _)| *cached == key) {
            let lines = self.translate_song(song, source, target);
            if self.songs.len() >= CACHED_SONGS {
                self.songs.pop_front();
            }
            self.songs.push_back((key, lines));
        }
        self.songs
            .iter()
            .find(|(cached, _)| *cached == key)
            .and_then(|(_, lines)| lines.get(index).cloned().flatten())
            .unwrap_or_default()
    }

    fn translate_song(&mut self, song: &str, source: &str, target: &str) -> Vec<Option<String>> {
        let lines: Vec<String> = song
            .split('\n')
            .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
            .collect();
        let mut result = vec![None; lines.len()];
        if lines.len() > MAX_LINES {
            self.warn("song exceeds the translation line limit");
            return result;
        }
        let mut unique: Vec<&str> = Vec::new();
        for line in &lines {
            if !line.is_empty() && !unique.contains(&line.as_str()) {
                unique.push(line);
            }
        }
        let context = truncated(song, MAX_BATCH_BYTES);
        let mut translated: HashMap<&str, String> = HashMap::new();
        for batch in unique.chunks(BATCH) {
            if batch.iter().map(|text| text.len()).sum::<usize>() > MAX_BATCH_BYTES {
                continue;
            }
            match self.request(batch, source, target, context) {
                Ok(values) => {
                    for (text, value) in batch.iter().zip(values) {
                        if let Some(value) = value.filter(|value| !value.trim().is_empty()) {
                            translated.insert(text, value);
                        }
                    }
                }
                Err(reason) => {
                    self.warn(&reason);
                    // A worker that failed mid-song is restarted for the next request.
                    self.worker = None;
                    break;
                }
            }
        }
        for (slot, line) in result.iter_mut().zip(&lines) {
            *slot = translated.get(line.as_str()).cloned();
        }
        result
    }

    fn request(
        &mut self,
        texts: &[&str],
        source: &str,
        target: &str,
        context: &str,
    ) -> Result<Vec<Option<String>>, String> {
        if self.worker.is_none() {
            self.worker = Some(Worker::start(
                &self.host,
                &self.worker_dir,
                &self.model_dir,
            )?);
        }
        let worker = self
            .worker
            .as_mut()
            .ok_or("translation worker unavailable")?;
        let request = serde_json::json!({
            "texts": texts, "source": source, "target": target, "context": context,
        });
        let mut bytes = serde_json::to_vec(&request).map_err(|error| error.to_string())?;
        bytes.push(b'\n');
        worker
            .input
            .write_all(&bytes)
            .and_then(|()| worker.input.flush())
            .map_err(|error| format!("translation worker input failed: {error}"))?;
        let reply = worker
            .replies
            .recv_timeout(REPLY_TIMEOUT)
            .map_err(|error| match error {
                mpsc::RecvTimeoutError::Timeout => "translation worker did not respond".to_string(),
                mpsc::RecvTimeoutError::Disconnected => {
                    "translation worker exited; see the log for its errors".to_string()
                }
            })?;
        let reply: serde_json::Value =
            serde_json::from_str(&reply).map_err(|_| "invalid translation reply".to_string())?;
        let Some(values) = reply.get("translations").and_then(|value| value.as_array()) else {
            return Err(reply
                .get("error")
                .and_then(|value| value.as_str())
                .unwrap_or("translation worker returned no translations")
                .to_string());
        };
        if values.len() != texts.len() {
            return Err("translation changed the number of lines".to_string());
        }
        Ok(values
            .iter()
            .map(|value| value.as_str().map(str::to_string))
            .collect())
    }

    fn warn(&self, message: &str) {
        self.host
            .log()
            .write(LOG_WARN, &format!("Lyrics translation: {message}"));
    }
}

fn song_key(song: &str, source: &str, target: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    (song, source, target).hash(&mut hasher);
    hasher.finish()
}

fn truncated(text: &str, max_bytes: usize) -> &str {
    let mut end = text.len().min(max_bytes);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

struct Worker {
    process: Child,
    input: ChildStdin,
    replies: mpsc::Receiver<String>,
    readers: Vec<JoinHandle<()>>,
}

impl Worker {
    fn start(host: &Host, worker_dir: &Path, model_dir: &Path) -> Result<Self, String> {
        let python = worker_dir.join("runtime").join("pythonw.exe");
        let script = worker_dir.join("translate.py");
        if !python.is_file() || !script.is_file() {
            return Err(format!(
                "translation runtime is missing from {}",
                worker_dir.display()
            ));
        }
        if !model_dir.join("model.bin").is_file() {
            return Err(format!(
                "NLLB model is missing from {}",
                model_dir.display()
            ));
        }
        let mut process = Command::new(python)
            .arg("-I")
            .arg("-u")
            .arg(script)
            .arg(model_dir)
            .current_dir(worker_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("cannot start the translation runtime: {error}"))?;
        let (Some(input), Some(output), Some(errors)) = (
            process.stdin.take(),
            process.stdout.take(),
            process.stderr.take(),
        ) else {
            stop(&mut process);
            return Err("translation worker pipes are unavailable".to_string());
        };
        let (send, replies) = mpsc::channel();
        let mut readers = Vec::new();
        let output_reader = std::thread::Builder::new()
            .name("nllb-output".to_string())
            .spawn(move || {
                let mut reader = BufReader::new(output);
                loop {
                    let mut line = String::new();
                    match reader.by_ref().take(MAX_REPLY_BYTES).read_line(&mut line) {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {
                            if !line.ends_with('\n') || send.send(line).is_err() {
                                break;
                            }
                        }
                    }
                }
            });
        match output_reader {
            Ok(handle) => readers.push(handle),
            Err(error) => {
                stop(&mut process);
                return Err(format!("cannot read translation output: {error}"));
            }
        }
        // Startup failures such as a missing package are only reported on stderr.
        let log_host = host.clone();
        if let Ok(handle) = std::thread::Builder::new()
            .name("nllb-errors".to_string())
            .spawn(move || {
                let mut reader = BufReader::new(errors);
                let mut logged = 0;
                loop {
                    let mut line = String::new();
                    match reader.by_ref().take(MAX_REPLY_BYTES).read_line(&mut line) {
                        Ok(0) | Err(_) => break,
                        Ok(_) if logged < MAX_LOGGED_STDERR_LINES => {
                            logged += 1;
                            log_host.log().write(
                                LOG_WARN,
                                &format!("Lyrics translation worker: {}", line.trim_end()),
                            );
                        }
                        Ok(_) => {}
                    }
                }
            })
        {
            readers.push(handle);
        }
        Ok(Self {
            process,
            input,
            replies,
            readers,
        })
    }
}

fn stop(process: &mut Child) {
    if matches!(process.try_wait(), Ok(None)) {
        let _ = process.kill();
    }
    let _ = process.wait();
}

impl Drop for Worker {
    fn drop(&mut self) {
        stop(&mut self.process);
        // The pipes close with the process, so both readers finish.
        for reader in self.readers.drain(..) {
            let _ = reader.join();
        }
    }
}
