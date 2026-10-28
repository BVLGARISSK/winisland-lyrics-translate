"""Downloads the NLLB model ZIP, checks its SHA-256, and unpacks it into the model folder.

Usage: download_model.py <url> <sha256> <model folder>
Prints one status line at a time: `progress <done> <total>`, `verifying`, `extracting`,
`done`, or `error <message>`.
"""

import hashlib
import shutil
import sys
import tempfile
import urllib.request
import zipfile
from pathlib import Path

CHUNK = 1 << 20
MAX_BYTES = 2 * 1024 ** 3
REQUIRED = ("model.bin", "tokenizer.json", "config.json")


def report(line):
    sys.stdout.write(line + "\n")
    sys.stdout.flush()


def download(url, target):
    request = urllib.request.Request(url, headers={"User-Agent": "winisland-lyrics-translate"})
    digest = hashlib.sha256()
    with urllib.request.urlopen(request, timeout=60) as response, open(target, "wb") as out:
        total = int(response.headers.get("Content-Length") or 0)
        if total > MAX_BYTES:
            raise ValueError("model download is larger than expected")
        done = 0
        last = -1
        while chunk := response.read(CHUNK):
            done += len(chunk)
            if done > MAX_BYTES:
                raise ValueError("model download is larger than expected")
            out.write(chunk)
            digest.update(chunk)
            percent = done * 100 // total if total else 0
            if percent != last:
                last = percent
                report(f"progress {done} {total}")
    return digest.hexdigest()


def unpack(archive, destination):
    with zipfile.ZipFile(archive) as zip_file:
        for name in zip_file.namelist():
            path = Path(name)
            if path.is_absolute() or ".." in path.parts or len(path.parts) != 1:
                raise ValueError("model archive has an unexpected layout")
        zip_file.extractall(destination)
    missing = [name for name in REQUIRED if not (destination / name).is_file()]
    if missing:
        raise ValueError("model archive is missing " + ", ".join(missing))


def main():
    url, expected, target = sys.argv[1], sys.argv[2].lower(), Path(sys.argv[3])
    target.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix=".nllb-", dir=target.parent) as work:
        work = Path(work)
        archive = work / "model.zip"
        actual = download(url, archive)
        report("verifying")
        if actual != expected:
            raise ValueError("model download failed its checksum")
        report("extracting")
        unpacked = work / "model"
        unpacked.mkdir()
        unpack(archive, unpacked)
        archive.unlink()
        if target.exists():
            shutil.rmtree(target)
        unpacked.replace(target)
    report("done")


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        report(f"error {type(error).__name__}: {error}"[:300])
        sys.exit(1)
