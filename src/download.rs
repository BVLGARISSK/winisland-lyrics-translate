//! Downloads the NLLB model once, through the bundled Python runtime.

use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::thread::JoinHandle;

/// The model is too large for a plugin package, so it ships as a release asset.
const MODEL_URL: &str = "https://github.com/BVLGARISSK/winisland-lyrics-translate/releases/download/model-v1/nllb-200-distilled-600M-ct2-int8.zip";
const MODEL_SHA256: &str = "13c5e9cdc3ab6219d298fd4922cd9bb5fde7be22aba07836bf5fbfd9ae5e9f90";
/// Shown before the download starts; the release asset is about this size.
pub(crate) const MODEL_MEGABYTES: u64 = 560;

#[derive(Clone, PartialEq)]
pub(crate) enum DownloadState {
    Idle,
    Downloading { done: u64, total: u64 },
    Verifying,
    Extracting,
    Failed(String),
}

pub(crate) struct Download {
    process: Child,
    reader: Option<JoinHandle<()>>,
}

impl Download {
    /// Starts the download; `update` receives every state change, ending in `Idle` or `Failed`.
    pub(crate) fn start(
        worker_dir: &Path,
        model_dir: &Path,
        update: impl Fn(DownloadState) + Send + 'static,
    ) -> Result<Self, String> {
        let python = worker_dir.join("runtime").join("pythonw.exe");
        let script = worker_dir.join("download_model.py");
        if !python.is_file() || !script.is_file() {
            return Err(format!(
                "download helper is missing from {}",
                worker_dir.display()
            ));
        }
        let mut process = Command::new(python)
            .arg("-I")
            .arg("-u")
            .arg(script)
            .arg(MODEL_URL)
            .arg(MODEL_SHA256)
            .arg(model_dir)
            .current_dir(worker_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| format!("cannot start the download: {error}"))?;
        let Some(output) = process.stdout.take() else {
            let _ = process.kill();
            let _ = process.wait();
            return Err("download output is unavailable".to_string());
        };
        let reader = std::thread::Builder::new()
            .name("nllb-download".to_string())
            .spawn(move || {
                let mut finished = false;
                for line in BufReader::new(output).lines() {
                    let Ok(line) = line else {
                        break;
                    };
                    let state = parse(&line);
                    finished = matches!(state, DownloadState::Idle | DownloadState::Failed(_));
                    update(state);
                }
                if !finished {
                    update(DownloadState::Failed("the download stopped".to_string()));
                }
            });
        match reader {
            Ok(handle) => Ok(Self {
                process,
                reader: Some(handle),
            }),
            Err(error) => {
                let _ = process.kill();
                let _ = process.wait();
                Err(format!("cannot follow the download: {error}"))
            }
        }
    }
}

fn parse(line: &str) -> DownloadState {
    let mut parts = line.split_whitespace();
    match parts.next() {
        Some("progress") => {
            let mut number = || parts.next().and_then(|value| value.parse().ok());
            let done = number().unwrap_or(0);
            let total = number().unwrap_or(0);
            DownloadState::Downloading { done, total }
        }
        Some("verifying") => DownloadState::Verifying,
        Some("extracting") => DownloadState::Extracting,
        Some("done") => DownloadState::Idle,
        _ => DownloadState::Failed(
            line.strip_prefix("error ")
                .unwrap_or("unexpected download output")
                .to_string(),
        ),
    }
}

impl Drop for Download {
    fn drop(&mut self) {
        if matches!(self.process.try_wait(), Ok(None)) {
            let _ = self.process.kill();
        }
        let _ = self.process.wait();
        // The pipe closes with the process, so the reader finishes.
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}
