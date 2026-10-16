#!/usr/bin/env python3
"""Build a single HTML page that embeds PNG screenshots as data URIs.

The desktop app is verified by rendering frames with `--screenshot`, but the
agent (and this browser panel) cannot open a local file directly, so the frames
are inlined into one page instead. Development helper only: nothing in the app
depends on it.

    python3 tools/art/shot_viewer.py out.html shot-a.png shot-b.png
"""

import base64
import html
import os
import sys


def main(argv: list[str]) -> int:
    if len(argv) < 3:
        print(__doc__)
        return 2
    out_path, paths = argv[1], argv[2:]
    parts = [
        "<!doctype html><html><head><meta charset='utf-8'><title>frames</title><style>",
        "body{margin:0;background:#141414;font:14px system-ui;color:#eee}",
        "figure{margin:0 0 24px 0}",
        "figcaption{padding:8px 12px;background:#262626;color:#ffd166;font-weight:700}",
        "img{display:block;width:100%;height:auto}</style></head><body>",
    ]
    for path in paths:
        if not os.path.exists(path):
            print(f"missing: {path}")
            continue
        with open(path, "rb") as handle:
            blob = base64.b64encode(handle.read()).decode()
        name = html.escape(os.path.basename(path))
        parts.append(
            f"<figure><figcaption>{name} — {os.path.getsize(path)} bytes</figcaption>"
            f"<img src='data:image/png;base64,{blob}'></figure>"
        )
    parts.append("</body></html>")
    with open(out_path, "w") as handle:
        handle.write("\n".join(parts))
    print(f"wrote {out_path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
