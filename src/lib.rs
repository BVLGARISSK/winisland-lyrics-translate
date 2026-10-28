//! WinIsland plugin that shows an offline NLLB translation below each lyric line.

mod download;
mod translator;

use std::ffi::c_void;
use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, Weak};
use std::thread::JoinHandle;
use std::time::Duration;

use winisland_plugin_api::sdk::{Host, LyricsLine, Resource};
use winisland_plugin_api::settings::{
    SETTINGS_ITEM_BUTTON, SETTINGS_ITEM_GROUP_END, SETTINGS_ITEM_GROUP_START, SETTINGS_ITEM_LABEL,
    SETTINGS_ITEM_SECTION, SETTINGS_ITEM_SELECT, SettingsChangeV2, SettingsItemV2,
    SettingsOptionV2, SettingsPageDataV2,
};
use winisland_plugin_api::{
    ABI_VERSION_2, CAP_LYRICS, CAP_SETTINGS, CAP_STORE, CAP_SYSTEM, IFACE_SETTINGS,
    IFACE_VERSION_1, PluginCreateInfoV2, PluginDescriptorV2, PluginHandleV2, PluginHostV2,
    PluginMetadataC, PluginStatus, PluginToken, ResourceId, SettingsApiV2, TablePrefix,
    str_to_fixed,
};

use download::{Download, DownloadState, MODEL_MEGABYTES};
use translator::Translator;

const ID: &str = "winisland-lyrics-translate";
const PAGE_KEY: &str = "lyrics-translation";
const MODEL_FOLDER: &str = "nllb-200-distilled-600M";
const TARGETS: [&str; 5] = ["zh-TW", "zh-CN", "en", "ja", "ko"];
const LOG_ERROR: u32 = 3;

static DESCRIPTOR: PluginDescriptorV2 = PluginDescriptorV2 {
    struct_size: std::mem::size_of::<PluginDescriptorV2>() as u32,
    abi_version: ABI_VERSION_2,
    capabilities: CAP_LYRICS | CAP_SETTINGS | CAP_STORE | CAP_SYSTEM,
    metadata: PluginMetadataC::new(
        ID,
        "Lyrics Translation",
        env!("CARGO_PKG_VERSION"),
        "BVLGARISSK",
        "Offline lyric translation below each line with NLLB-200 Distilled 600M",
    ),
    create: Some(create),
    shutdown: Some(shutdown),
    destroy: Some(destroy),
    on_tick: None,
};

/// # Safety
/// WinIsland calls this exported symbol using the ABI v2 entry signature.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn winisland_plugin_entry_v2() -> *const PluginDescriptorV2 {
    &DESCRIPTOR
}

struct Labels {
    title: &'static str,
    target: &'static str,
    source: &'static str,
    auto: &'static str,
    ready: &'static str,
    missing: &'static str,
    toggle_hint: &'static str,
    language_hint: &'static str,
    download_row: &'static str,
    download_button: &'static str,
    downloading: &'static str,
    verifying: &'static str,
    extracting: &'static str,
    failed: &'static str,
    network_note: &'static str,
}

impl Labels {
    fn for_language(language: &str) -> Self {
        match language {
            "zh_tw" => Self {
                title: "歌詞翻譯",
                target: "翻譯語言",
                source: "歌詞原文語言",
                auto: "自動辨識",
                ready: "NLLB 模型已就緒，翻譯完全在本機離線進行。",
                missing: "找不到 NLLB 模型。可按「下載」取得，或手動放到：",
                toggle_hint: "開關：設定 > 音樂 > 顯示附加歌詞。",
                language_hint: "更改語言後，從下一首歌開始套用。",
                download_row: "NLLB 模型",
                download_button: "下載",
                downloading: "下載中",
                verifying: "正在校驗檔案…",
                extracting: "正在解壓縮…",
                failed: "下載失敗：",
                network_note: "只有下載模型時需要網路，之後翻譯完全離線。",
            },
            "zh_cn" => Self {
                title: "歌词翻译",
                target: "翻译语言",
                source: "歌词原文语言",
                auto: "自动识别",
                ready: "NLLB 模型已就绪，翻译完全在本机离线进行。",
                missing: "找不到 NLLB 模型。可点“下载”获取，或手动放到：",
                toggle_hint: "开关：设置 > 音乐 > 显示附加歌词。",
                language_hint: "更改语言后，从下一首歌开始生效。",
                download_row: "NLLB 模型",
                download_button: "下载",
                downloading: "下载中",
                verifying: "正在校验文件…",
                extracting: "正在解压缩…",
                failed: "下载失败：",
                network_note: "只有下载模型时需要网络，之后翻译完全离线。",
            },
            "ja_jp" => Self {
                title: "歌詞の翻訳",
                target: "翻訳先の言語",
                source: "歌詞の原文の言語",
                auto: "自動検出",
                ready: "NLLB モデルの準備ができています。翻訳はすべてオフラインで行います。",
                missing: "NLLB モデルが見つかりません。「ダウンロード」で取得するか、次の場所に置いてください：",
                toggle_hint: "オン／オフ：設定 > ミュージック > サブ歌詞を表示。",
                language_hint: "言語の変更は次の曲から適用されます。",
                download_row: "NLLB モデル",
                download_button: "ダウンロード",
                downloading: "ダウンロード中",
                verifying: "ファイルを検証中…",
                extracting: "展開中…",
                failed: "ダウンロードに失敗しました：",
                network_note: "ネットワークを使うのはモデルのダウンロード時だけで、翻訳は完全にオフラインです。",
            },
            "ko_kr" => Self {
                title: "가사 번역",
                target: "번역 언어",
                source: "가사 원문 언어",
                auto: "자동 감지",
                ready: "NLLB 모델이 준비되었습니다. 번역은 모두 오프라인으로 진행됩니다.",
                missing: "NLLB 모델을 찾을 수 없습니다. ‘다운로드’를 누르거나 다음 위치에 두세요:",
                toggle_hint: "켜기/끄기: 설정 > 음악 > 보조 가사 표시.",
                language_hint: "언어 변경은 다음 곡부터 적용됩니다.",
                download_row: "NLLB 모델",
                download_button: "다운로드",
                downloading: "다운로드 중",
                verifying: "파일 확인 중…",
                extracting: "압축 해제 중…",
                failed: "다운로드 실패: ",
                network_note: "네트워크는 모델을 다운로드할 때만 사용하며, 번역은 완전히 오프라인으로 진행됩니다.",
            },
            _ => Self {
                title: "Lyrics Translation",
                target: "Translate into",
                source: "Lyrics language",
                auto: "Detect automatically",
                ready: "The NLLB model is ready; translation runs fully offline.",
                missing: "The NLLB model is missing. Press Download, or place it in:",
                toggle_hint: "Turn it on in Settings > Music > Show Secondary Lyrics.",
                language_hint: "A language change applies from the next song.",
                download_row: "NLLB model",
                download_button: "Download",
                downloading: "Downloading",
                verifying: "Verifying the file…",
                extracting: "Unpacking…",
                failed: "Download failed: ",
                network_note: "Only the model download uses the network; translation stays fully offline.",
            },
        }
    }
}

fn language_name(code: &str) -> &'static str {
    match code {
        "zh-TW" => "繁體中文",
        "zh-CN" => "简体中文",
        "en" => "English",
        "ja" => "日本語",
        "ko" => "한국어",
        _ => "",
    }
}

fn default_target(ui_language: &str) -> &'static str {
    match ui_language {
        "zh_tw" => "zh-TW",
        "zh_cn" => "zh-CN",
        "ja_jp" => "ja",
        "ko_kr" => "ko",
        _ => "en",
    }
}

#[derive(Clone)]
struct Choices {
    target: String,
    source: String,
}

struct Shared {
    /// Lets settings callbacks, which only get `&Shared`, hand the state to new threads.
    this: Weak<Shared>,
    host: Host,
    /// Raw host table for the settings service, which the SDK does not wrap interactively.
    host_api: usize,
    token: PluginToken,
    labels: Labels,
    choices: Mutex<Choices>,
    translator: Mutex<Translator>,
    page: Mutex<Option<ResourceId>>,
    /// Wakes the settings updater thread; cleared at shutdown so the thread exits.
    refresh: Mutex<Option<Sender<()>>>,
    worker_dir: PathBuf,
    model_dir: PathBuf,
    download_state: Mutex<DownloadState>,
    download: Mutex<Option<Download>>,
}

struct Instance {
    shared: Arc<Shared>,
    lyrics: Option<Resource>,
    updater: Option<JoinHandle<()>>,
}

fn plugin_paths() -> Option<(PathBuf, PathBuf)> {
    let roaming = std::env::var_os("APPDATA")?;
    let local = std::env::var_os("LOCALAPPDATA")?;
    let worker = PathBuf::from(roaming)
        .join("WinIsland")
        .join("plugins")
        .join(ID)
        .join("worker");
    let model = PathBuf::from(local)
        .join("WinIsland")
        .join("models")
        .join(MODEL_FOLDER);
    Some((worker, model))
}

fn stored(host: &Host, key: &str, allowed: &[&str]) -> Option<String> {
    let bytes = host.store().ok()?.get(key).ok()??;
    let value = String::from_utf8(bytes).ok()?;
    allowed.contains(&value.as_str()).then_some(value)
}

fn fixed_text(bytes: &[u8]) -> &str {
    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    std::str::from_utf8(&bytes[..end]).unwrap_or_default()
}

fn option(value: &str, label: &str) -> SettingsOptionV2 {
    SettingsOptionV2 {
        value: str_to_fixed(value),
        label: str_to_fixed(label),
        ..Default::default()
    }
}

fn item(kind: u32, key: &str, label: &str, value: &str) -> SettingsItemV2 {
    SettingsItemV2 {
        kind,
        key: str_to_fixed(key),
        label: str_to_fixed(label),
        value: str_to_fixed(value),
        ..Default::default()
    }
}

/// # Safety
/// `host_api` must be the live host table this instance received in `create`.
unsafe fn settings_table(host_api: usize) -> Option<SettingsApiV2> {
    // SAFETY: The caller guarantees the table is live for this instance.
    let host = unsafe { (host_api as *const PluginHostV2).as_ref() }?;
    let query = host.query?;
    // SAFETY: The host query accepts its own context and an interface id.
    let pointer = unsafe { query(host.prefix.context, IFACE_SETTINGS, IFACE_VERSION_1) };
    if pointer.is_null() {
        return None;
    }
    // SAFETY: Every service table begins with a readable prefix.
    let prefix = unsafe { std::ptr::read_unaligned(pointer.cast::<TablePrefix>()) };
    if prefix.struct_size < std::mem::size_of::<SettingsApiV2>() as u32
        || prefix.version != IFACE_VERSION_1
    {
        return None;
    }
    // SAFETY: The checked size covers the whole table.
    Some(unsafe { std::ptr::read_unaligned(pointer.cast::<SettingsApiV2>()) })
}

impl Shared {
    /// Creates the settings page, or replaces its contents when it already exists.
    fn publish_page(self: &Arc<Self>) -> Result<(), PluginStatus> {
        let choices = self
            .choices
            .lock()
            .map_err(|_| PluginStatus::Internal)?
            .clone();
        let (ready, model_dir) = {
            let translator = self.translator.lock().map_err(|_| PluginStatus::Internal)?;
            (
                translator.model_ready(),
                translator.model_dir().display().to_string(),
            )
        };
        let labels = &self.labels;
        let targets: Vec<_> = TARGETS
            .iter()
            .map(|code| option(code, language_name(code)))
            .collect();
        let sources: Vec<_> = std::iter::once(option("auto", labels.auto))
            .chain(TARGETS.iter().map(|code| option(code, language_name(code))))
            .collect();
        let mut target = item(
            SETTINGS_ITEM_SELECT,
            "target",
            labels.target,
            &choices.target,
        );
        target.options = targets.as_ptr();
        target.option_count = targets.len() as u32;
        let mut source = item(
            SETTINGS_ITEM_SELECT,
            "source",
            labels.source,
            &choices.source,
        );
        source.options = sources.as_ptr();
        source.option_count = sources.len() as u32;
        let mut items = vec![
            item(SETTINGS_ITEM_SECTION, "", labels.title, ""),
            item(SETTINGS_ITEM_GROUP_START, "", "", ""),
            target,
            source,
        ];
        let download_state = self
            .download_state
            .lock()
            .map_err(|_| PluginStatus::Internal)?
            .clone();
        let button_label = format!("{} ({MODEL_MEGABYTES} MB)", labels.download_button);
        match download_state {
            _ if ready => items.push(item(SETTINGS_ITEM_LABEL, "", labels.ready, "")),
            DownloadState::Downloading { done, total } => {
                let megabytes = |bytes: u64| bytes / (1024 * 1024);
                let status = match done.saturating_mul(100).checked_div(total) {
                    Some(percent) => format!(
                        "{} {percent}% ({} / {} MB)",
                        labels.downloading,
                        megabytes(done),
                        megabytes(total)
                    ),
                    None => format!("{} ({} MB)", labels.downloading, megabytes(done)),
                };
                items.push(item(SETTINGS_ITEM_LABEL, "", &status, ""));
            }
            DownloadState::Verifying => {
                items.push(item(SETTINGS_ITEM_LABEL, "", labels.verifying, ""));
            }
            DownloadState::Extracting => {
                items.push(item(SETTINGS_ITEM_LABEL, "", labels.extracting, ""));
            }
            DownloadState::Idle | DownloadState::Failed(_) => {
                if let DownloadState::Failed(reason) = &download_state {
                    let failed = format!("{}{reason}", labels.failed);
                    items.push(item(SETTINGS_ITEM_LABEL, "", &failed, ""));
                }
                items.push(item(SETTINGS_ITEM_LABEL, "", labels.missing, ""));
                items.push(item(SETTINGS_ITEM_LABEL, "", &model_dir, ""));
                items.push(item(
                    SETTINGS_ITEM_BUTTON,
                    "download",
                    labels.download_row,
                    &button_label,
                ));
                items.push(item(SETTINGS_ITEM_LABEL, "", labels.network_note, ""));
            }
        }
        items.push(item(SETTINGS_ITEM_LABEL, "", labels.toggle_hint, ""));
        items.push(item(SETTINGS_ITEM_LABEL, "", labels.language_hint, ""));
        items.push(item(SETTINGS_ITEM_GROUP_END, "", "", ""));
        let data = SettingsPageDataV2 {
            key: str_to_fixed(PAGE_KEY),
            title: str_to_fixed(labels.title),
            items: items.as_ptr(),
            item_count: items.len() as u32,
            on_change: Some(on_settings_change),
            callback_data: Arc::as_ptr(self).cast_mut().cast(),
            ..Default::default()
        };
        // SAFETY: `host_api` is this instance's live host table.
        let table = unsafe { settings_table(self.host_api) }.ok_or(PluginStatus::Internal)?;
        let mut page = self.page.lock().map_err(|_| PluginStatus::Internal)?;
        let status = match *page {
            Some(id) => {
                let update = table.update.ok_or(PluginStatus::Internal)?;
                // SAFETY: Items and options outlive this call, which copies them.
                unsafe { update(table.prefix.context, self.token, id, &data) }
            }
            None => {
                let create = table.create.ok_or(PluginStatus::Internal)?;
                let mut id = ResourceId::INVALID;
                // SAFETY: Items and options outlive this call, which copies them.
                let status = unsafe { create(table.prefix.context, self.token, &data, &mut id) };
                if status == PluginStatus::Ok {
                    *page = Some(id);
                }
                status
            }
        };
        if status == PluginStatus::Ok {
            Ok(())
        } else {
            Err(status)
        }
    }

    fn request_refresh(&self) {
        if let Ok(refresh) = self.refresh.lock()
            && let Some(refresh) = refresh.as_ref()
        {
            let _ = refresh.send(());
        }
    }

    /// Starts the one-time model download unless it is running or the model is already present.
    fn start_download(&self) {
        let Some(this) = self.this.upgrade() else {
            return;
        };
        let Ok(mut download) = self.download.lock() else {
            return;
        };
        let running = matches!(
            self.download_state.lock().map(|state| state.clone()),
            Ok(DownloadState::Downloading { .. }
                | DownloadState::Verifying
                | DownloadState::Extracting)
        );
        if running || self.model_dir.join("model.bin").is_file() {
            return;
        }
        let started = Download::start(&self.worker_dir, &self.model_dir, move |state| {
            if let DownloadState::Failed(reason) = &state {
                this.host.log().write(
                    LOG_ERROR,
                    &format!("Lyrics translation model download failed: {reason}"),
                );
            }
            if let Ok(mut current) = this.download_state.lock() {
                *current = state;
            }
            this.request_refresh();
        });
        let state = match started {
            Ok(handle) => {
                *download = Some(handle);
                DownloadState::Downloading { done: 0, total: 0 }
            }
            Err(reason) => DownloadState::Failed(reason),
        };
        if let Ok(mut current) = self.download_state.lock() {
            *current = state;
        }
        drop(download);
        self.request_refresh();
    }

    fn release_page(&self) {
        let Ok(mut page) = self.page.lock() else {
            return;
        };
        let Some(id) = page.take() else {
            return;
        };
        // SAFETY: `host_api` is this instance's live host table.
        if let Some(release) = unsafe { settings_table(self.host_api) }
            .and_then(|t| t.release.map(|release| (release, t.prefix.context)))
        {
            // SAFETY: The page belongs to this plugin and no settings callback is running.
            let _ = unsafe { (release.0)(release.1, self.token, id) };
        }
    }

    fn translate(&self, line: &LyricsLine<'_>) -> String {
        let Ok(choices) = self.choices.lock().map(|choices| choices.clone()) else {
            return String::new();
        };
        let Ok(index) = usize::try_from(line.index) else {
            return String::new();
        };
        match self.translator.lock() {
            Ok(mut translator) => {
                translator.line(line.song, index, &choices.source, &choices.target)
            }
            Err(_) => String::new(),
        }
    }
}

unsafe extern "C" fn on_settings_change(
    callback_data: *mut c_void,
    _page: ResourceId,
    change: *const SettingsChangeV2,
) -> PluginStatus {
    if callback_data.is_null() || change.is_null() {
        return PluginStatus::InvalidArgument;
    }
    // SAFETY: The page is released before the shared state it points to is dropped.
    let shared = unsafe { &*callback_data.cast::<Shared>() };
    // SAFETY: The host keeps the change record alive for this callback.
    let change = unsafe { &*change };
    let key = fixed_text(&change.key);
    let value = fixed_text(&change.value);
    if key == "download" {
        shared.start_download();
        return PluginStatus::Ok;
    }
    let valid = match key {
        "target" => TARGETS.contains(&value),
        "source" => value == "auto" || TARGETS.contains(&value),
        _ => false,
    };
    if !valid {
        return PluginStatus::InvalidArgument;
    }
    let Ok(mut choices) = shared.choices.lock() else {
        return PluginStatus::Internal;
    };
    if key == "target" {
        choices.target = value.to_string();
    } else {
        choices.source = value.to_string();
    }
    drop(choices);
    shared.request_refresh();
    if let Ok(store) = shared.host.store()
        && let Err(error) = store.set(key, value.as_bytes())
    {
        shared.host.log().write(
            LOG_ERROR,
            &format!("Cannot save lyrics translation setting: {error}"),
        );
    }
    PluginStatus::Ok
}

/// Republishes the settings page after each change; the page cannot be updated while its own
/// change callback is still running, so this happens on a separate thread with retries.
fn spawn_updater(shared: Arc<Shared>) -> Option<JoinHandle<()>> {
    let (send, receive) = mpsc::channel::<()>();
    *shared.refresh.lock().ok()? = Some(send);
    std::thread::Builder::new()
        .name("lyrics-translate-settings".to_string())
        .spawn(move || {
            while receive.recv().is_ok() {
                for _ in 0..40 {
                    match shared.publish_page() {
                        Err(PluginStatus::LimitExceeded) => {
                            std::thread::sleep(Duration::from_millis(50));
                        }
                        _ => break,
                    }
                }
            }
        })
        .ok()
}

unsafe extern "C" fn create(
    info: *const PluginCreateInfoV2,
    out_handle: *mut PluginHandleV2,
) -> PluginStatus {
    if info.is_null() || out_handle.is_null() {
        return PluginStatus::InvalidArgument;
    }
    // SAFETY: WinIsland supplies a readable create-info header.
    let info = unsafe { &*info };
    if info.struct_size < std::mem::size_of::<PluginCreateInfoV2>() as u32
        || info.abi_version != ABI_VERSION_2
        || info.plugin_token == PluginToken::INVALID
    {
        return PluginStatus::UnsupportedVersion;
    }
    // SAFETY: The host table remains allocated throughout this instance's lifetime.
    let host = match unsafe { Host::from_raw(info.host_api, info.plugin_token) } {
        Ok(host) => host,
        Err(_) => return PluginStatus::InvalidArgument,
    };
    let Some((worker_dir, model_dir)) = plugin_paths() else {
        host.log().write(
            LOG_ERROR,
            "Lyrics translation: APPDATA or LOCALAPPDATA is not set",
        );
        return PluginStatus::Internal;
    };
    let ui_language = host
        .system()
        .and_then(|system| system.current_language())
        .unwrap_or_default();
    let sources: Vec<&str> = std::iter::once("auto").chain(TARGETS).collect();
    let choices = Choices {
        target: stored(&host, "target", &TARGETS)
            .unwrap_or_else(|| default_target(&ui_language).to_string()),
        source: stored(&host, "source", &sources).unwrap_or_else(|| "auto".to_string()),
    };
    let shared = Arc::new_cyclic(|this| Shared {
        this: this.clone(),
        host: host.clone(),
        host_api: info.host_api as usize,
        token: info.plugin_token,
        labels: Labels::for_language(&ui_language),
        choices: Mutex::new(choices),
        translator: Mutex::new(Translator::new(
            host.clone(),
            worker_dir.clone(),
            model_dir.clone(),
        )),
        page: Mutex::new(None),
        refresh: Mutex::new(None),
        worker_dir,
        model_dir,
        download_state: Mutex::new(DownloadState::Idle),
        download: Mutex::new(None),
    });
    if let Err(status) = shared.publish_page() {
        host.log().write(
            LOG_ERROR,
            &format!("Lyrics translation settings page failed: {status:?}"),
        );
    }
    let translate_target = shared.clone();
    let lyrics = match host
        .lyrics()
        .and_then(|api| api.register_secondary(move |line| translate_target.translate(line)))
    {
        Ok(resource) => resource,
        Err(error) => {
            host.log().write(
                LOG_ERROR,
                &format!("Lyrics translation needs a WinIsland with secondary lyrics: {error}"),
            );
            shared.release_page();
            return PluginStatus::UnsupportedVersion;
        }
    };
    let updater = spawn_updater(shared.clone());
    let instance = Box::new(Instance {
        shared,
        lyrics: Some(lyrics),
        updater,
    });
    // SAFETY: WinIsland treats this pointer as opaque until destroy.
    unsafe { out_handle.write(Box::into_raw(instance).cast::<c_void>()) };
    PluginStatus::Ok
}

unsafe extern "C" fn shutdown(handle: PluginHandleV2) -> PluginStatus {
    if handle.is_null() {
        return PluginStatus::InvalidArgument;
    }
    // SAFETY: This handle was created above and has not been destroyed.
    let instance = unsafe { &mut *handle.cast::<Instance>() };
    drop(instance.lyrics.take());
    // Stopping the download joins its reader, which may still post one last refresh.
    let download = instance
        .shared
        .download
        .lock()
        .ok()
        .and_then(|mut download| download.take());
    drop(download);
    if let Ok(mut refresh) = instance.shared.refresh.lock() {
        refresh.take();
    }
    if let Some(updater) = instance.updater.take() {
        let _ = updater.join();
    }
    instance.shared.release_page();
    if let Ok(mut translator) = instance.shared.translator.lock() {
        translator.stop();
    }
    PluginStatus::Ok
}

unsafe extern "C" fn destroy(handle: PluginHandleV2) {
    if !handle.is_null() {
        // SAFETY: WinIsland calls destroy once after successful shutdown.
        unsafe { drop(Box::from_raw(handle.cast::<Instance>())) };
    }
}
