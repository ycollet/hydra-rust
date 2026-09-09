#!/usr/bin/env python3
"""Fetches the public hydra-synth sketch database and decodes it into
individual .hydra files, for conformance-testing hydra-rust against a large
corpus of real-world sketches (see examples/check_corpus.rs).

The sketches themselves are third-party, user-submitted content (each one
carries a "// licensed with CC BY-NC-SA 4.0" header baked in by the editor)
and are NOT part of this repository - this script re-downloads them into a
gitignored directory instead.

Usage:
    python3 scripts/fetch_corpus.py [output-dir]   # default: ./sketches
"""
import base64
import json
import re
import sys
import urllib.parse
import urllib.request
from pathlib import Path

API_URL = "https://api.hydrasynth.xyz/sketches"
SAFE_ID_RE = re.compile(r"[^A-Za-z0-9_-]")


def decode_code(b64: str) -> str:
    raw = base64.b64decode(b64)
    return urllib.parse.unquote(raw.decode("ascii"))


def safe_filename(sketch_id: str) -> str:
    return SAFE_ID_RE.sub("_", sketch_id) or "unknown"


def main():
    out_dir = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("sketches")
    out_dir.mkdir(parents=True, exist_ok=True)

    print(f"fetching {API_URL} ...")
    with urllib.request.urlopen(API_URL) as resp:
        entries = json.load(resp)
    print(f"downloaded {len(entries)} entries")

    ok = 0
    failed = 0
    for entry in entries:
        sketch_id = entry.get("sketch_id") or entry.get("_id")
        code_b64 = entry.get("code")
        if not sketch_id or not code_b64:
            failed += 1
            continue
        try:
            code = decode_code(code_b64)
        except Exception:
            failed += 1
            continue

        fname = safe_filename(sketch_id)
        (out_dir / f"{fname}.hydra").write_text(code, encoding="utf-8")
        ok += 1

    print(f"wrote {ok} .hydra files to {out_dir}/ ({failed} skipped: empty/undecodable)")


if __name__ == "__main__":
    sys.exit(main())
