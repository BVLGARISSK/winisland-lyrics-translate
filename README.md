# Lyrics Translation

Shows an offline translation below each lyric line in WinIsland, using
[NLLB-200 Distilled 600M](https://huggingface.co/facebook/nllb-200-distilled-600M) through
CTranslate2. Lyrics never leave the computer, and no account is needed.

## Requirements

- A WinIsland build whose plugin API supports secondary-text lyric transformers
  (`LYRICS_TRANSFORMER_FLAG_SECONDARY`).
- The NLLB model converted to CTranslate2 int8 (about 560 MB). It is too large for a plugin
  package, so it is downloaded separately: press **Download** on the plugin's settings page, or
  download `nllb-200-distilled-600M-ct2-int8.zip` from this repository's releases and unpack it
  into `%LOCALAPPDATA%\WinIsland\models\nllb-200-distilled-600M`. The plugin checks the
  download's SHA-256 before unpacking it. Only this download uses the network.

## Use

1. Install the plugin ZIP by dropping it onto the island.
2. Turn on **Settings > Music > Show Secondary Lyrics**; turning it off stops new translations.
3. Pick the target language (Traditional Chinese, Simplified Chinese, English, Japanese,
   Korean) and the lyrics language (or automatic detection) on the plugin's settings page.

The original line stays on top with its line and word timing; the translation appears below
it once the whole song is translated, usually a few seconds after the lyrics appear.
Between Traditional and Simplified Chinese, the plugin converts the script with OpenCC instead
of rewording the line.

## License

The plugin code is GPL-3.0. The NLLB-200 model is by Meta AI under
[CC BY-NC 4.0](https://creativecommons.org/licenses/by-nc/4.0/): non-commercial use only, with
attribution. The model archive includes this notice as `MODEL-LICENSE.txt`.

---

# 歌詞翻譯

在 WinIsland 的每行歌詞下方顯示離線翻譯。使用
[NLLB-200 Distilled 600M](https://huggingface.co/facebook/nllb-200-distilled-600M)（CTranslate2 格式），
歌詞不會離開電腦，也不需要登入。

## 需求

- 外掛 API 支援「第二行歌詞轉換器」（`LYRICS_TRANSFORMER_FLAG_SECONDARY`）的 WinIsland。
- 轉成 CTranslate2 int8 的 NLLB 模型（約 560 MB）。模型太大，無法放進外掛套件，需要另外下載：
  在外掛設定頁按「下載」，或從本 repo 的 Releases 下載 `nllb-200-distilled-600M-ct2-int8.zip`，
  解壓縮到 `%LOCALAPPDATA%\WinIsland\models\nllb-200-distilled-600M`。外掛會先校驗 SHA-256
  再解壓縮。只有下載模型這一步需要網路。

## 使用方式

1. 把外掛 ZIP 拖到動態島上安裝。
2. 開啟「設定 > 音樂 > 顯示附加歌詞」；關閉後就不會再進行新的翻譯。
3. 在外掛的設定頁選擇翻譯語言（繁中、簡中、英、日、韓）和歌詞原文語言（或自動辨識）。

原文維持在上方，逐行與逐字時間都不變；整首翻譯完成後（通常在歌詞出現後幾秒），譯文會顯示在下方。
繁體與簡體中文之間，外掛用 OpenCC 轉換字形，不會改寫句子。

## 授權

外掛程式碼採用 GPL-3.0。NLLB-200 模型由 Meta AI 發布，授權為
[CC BY-NC 4.0](https://creativecommons.org/licenses/by-nc/4.0/)：僅限非商業用途，並須標示出處。
模型壓縮檔內附 `MODEL-LICENSE.txt` 說明。
