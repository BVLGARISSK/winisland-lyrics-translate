"""NLLB lyric translation over local JSON lines. No server or network access."""

import json
import os
from pathlib import Path
import re
import sys

os.environ["HF_HUB_OFFLINE"] = "1"
os.environ["TRANSFORMERS_OFFLINE"] = "1"
os.environ["HF_HUB_DISABLE_TELEMETRY"] = "1"
os.environ["TOKENIZERS_PARALLELISM"] = "false"


def deny_network(event, _args):
    if event in ("socket.connect", "socket.getaddrinfo", "socket.sendto"):
        raise OSError("Network access is disabled in the offline translator")


sys.addaudithook(deny_network)

import ctranslate2
from langid.langid import LanguageIdentifier, model as language_model
from tokenizers import Tokenizer
from opencc import OpenCC


MODEL = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(__file__).resolve().parent / "model"
LANGUAGES = {
    "en": "eng_Latn",
    "zh-TW": "zho_Hant",
    "zh-CN": "zho_Hans",
    "ja": "jpn_Jpan",
    "ko": "kor_Hang",
}
CHINESE = ("zho_Hans", "zho_Hant")
DETECTED = {
    "en": "eng_Latn", "zh": "zho_Hans", "ja": "jpn_Jpan", "ko": "kor_Hang",
    "es": "spa_Latn", "fr": "fra_Latn", "de": "deu_Latn", "it": "ita_Latn",
    "pt": "por_Latn", "ru": "rus_Cyrl", "uk": "ukr_Cyrl", "pl": "pol_Latn",
    "nl": "nld_Latn", "sv": "swe_Latn", "da": "dan_Latn", "nb": "nob_Latn",
    "fi": "fin_Latn", "cs": "ces_Latn", "ro": "ron_Latn", "hu": "hun_Latn",
    "tr": "tur_Latn", "el": "ell_Grek", "ar": "arb_Arab", "he": "heb_Hebr",
    "hi": "hin_Deva", "th": "tha_Thai", "vi": "vie_Latn", "id": "ind_Latn",
    "ms": "zsm_Latn", "tl": "tgl_Latn", "fa": "pes_Arab", "bn": "ben_Beng",
    "ta": "tam_Taml", "te": "tel_Telu", "ur": "urd_Arab", "bg": "bul_Cyrl",
    "hr": "hrv_Latn", "sr": "srp_Cyrl", "sk": "slk_Latn", "sl": "slv_Latn",
}


class Translator:
    def __init__(self):
        self.identifier = LanguageIdentifier.from_modelstring(language_model, norm_probs=True)
        self.simplifier = OpenCC("t2s")
        self.traditional = OpenCC("s2twp")
        self.tokenizer = Tokenizer.from_file(str(MODEL / "tokenizer.json"))
        self.translator = ctranslate2.Translator(
            str(MODEL), device="cpu", compute_type="int8",
            inter_threads=1, intra_threads=min(4, os.cpu_count() or 1),
        )

    def source_language(self, source, context):
        if source in LANGUAGES:
            return LANGUAGES[source]
        if source != "auto":
            raise ValueError("Unsupported source language")
        if re.search(r"[\u3040-\u30ff]", context):
            return "jpn_Jpan"
        if re.search(r"[\uac00-\ud7af]", context):
            return "kor_Hang"
        code, _confidence = self.identifier.classify(context[:24000])
        if code == "zh" and self.simplifier.convert(context) != context:
            return "zho_Hant"
        if code not in DETECTED:
            raise ValueError("Choose the source language manually")
        return DETECTED[code]

    def translate(self, request):
        texts = request.get("texts")
        if not isinstance(texts, list) or not 1 <= len(texts) <= 8:
            raise ValueError("Invalid batch size")
        if any(not isinstance(text, str) or not text.strip() for text in texts):
            raise ValueError("Invalid lyric text")
        if sum(len(text.encode("utf-8")) for text in texts) > 24000:
            raise ValueError("Lyric batch is too large")
        target = LANGUAGES[request["target"]]
        context = request.get("context", "\n".join(texts))
        if not isinstance(context, str) or len(context.encode("utf-8")) > 24000:
            raise ValueError("Invalid language detection context")
        source = self.source_language(request.get("source", "auto"), context)
        if source == target:
            return texts, source
        if source in CHINESE and target in CHINESE:
            # Between Chinese scripts the model would paraphrase; convert characters instead.
            converter = self.traditional if target == "zho_Hant" else self.simplifier
            return [converter.convert(text) for text in texts], source
        # A line that cannot be translated is returned as None so the rest of the song still is.
        output = [None] * len(texts)
        indices = []
        inputs = []
        for index, text in enumerate(texts):
            tokens = self.tokenizer.encode(text, add_special_tokens=False).tokens
            if len(tokens) > 510:
                continue
            indices.append(index)
            inputs.append([source] + tokens + ["</s>"])
        if not inputs:
            return output, source
        results = self.translator.translate_batch(
            inputs, target_prefix=[[target] for _ in inputs], beam_size=2,
            max_batch_size=8, max_decoding_length=256, no_repeat_ngram_size=3,
        )
        for index, result in zip(indices, results):
            tokens = result.hypotheses[0][1:]
            ids = [self.tokenizer.token_to_id(token) for token in tokens]
            if any(token is None for token in ids):
                continue
            text = self.tokenizer.decode(ids, skip_special_tokens=True).strip()
            if target == "zho_Hant":
                text = self.traditional.convert(text)
            elif target == "zho_Hans":
                text = self.simplifier.convert(text)
            output[index] = text or None
        return output, source


def main():
    sys.stdin.reconfigure(encoding="utf-8")
    sys.stdout.reconfigure(encoding="utf-8", newline="\n", line_buffering=True)
    translator = None
    for line in sys.stdin:
        try:
            if len(line.encode("utf-8")) > 512 * 1024:
                raise ValueError("Request is too large")
            request = json.loads(line)
            if translator is None:
                translator = Translator()
            texts, source = translator.translate(request)
            reply = {"translations": texts, "source": source, "device": "cpu",
                     "compute_type": "int8", "offline": True}
        except Exception as error:
            # The worker must outlive one bad request; the host logs the reason.
            reply = {"error": f"{type(error).__name__}: {error}"[:500]}
        sys.stdout.write(json.dumps(reply, ensure_ascii=False) + "\n")
        sys.stdout.flush()


if __name__ == "__main__":
    main()
