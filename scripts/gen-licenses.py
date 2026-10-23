#!/usr/bin/env python3
"""Regenerates THIRD_PARTY_LICENSES.txt from the crates Flint ships with (run after changing dependencies)."""
import hashlib, json, os, subprocess, sys

root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
meta = json.loads(subprocess.check_output(["cargo", "metadata", "--format-version", "1", "--locked"], cwd=root))
# only crates actually built for this machine's targets
resolve = {n["id"]: n for n in meta["resolve"]["nodes"]}
pkgs = {p["id"]: p for p in meta["packages"]}
seen, stack = set(), [meta["resolve"]["root"]]
while stack:
    i = stack.pop()
    if i in seen:
        continue
    seen.add(i)
    for d in resolve[i]["deps"]:
        if any(k.get("kind") in (None, "build") for k in d["dep_kinds"]):
            stack.append(d["pkg"])

texts, used_by, rows = {}, {}, []
for i in sorted(seen, key=lambda i: pkgs[i]["name"]):
    p = pkgs[i]
    if p["name"] == "flint":
        continue
    rows.append(f'{p["name"]} {p["version"]} — {p.get("license") or "see license file"}')
    d = os.path.dirname(p["manifest_path"])
    files = [f for f in sorted(os.listdir(d)) if f.upper().startswith(("LICENSE", "LICENCE", "COPYING", "NOTICE"))]
    for f in files:
        path = os.path.join(d, f)
        if os.path.isdir(path):
            continue
        t = open(path, encoding="utf-8", errors="replace").read().strip()
        h = hashlib.sha1(t.encode()).hexdigest()
        texts[h] = t
        used_by.setdefault(h, []).append(f'{p["name"]} {p["version"]}')
    if p["name"] == "epaint_default_fonts":
        for f in ["UFL.txt", "OFL.txt", "emoji-icon-font-mit-license.txt", "Hack-Regular.txt", "egui-icons.txt"]:
            path = os.path.join(d, "fonts", f)
            if os.path.exists(path):
                t = open(path, encoding="utf-8", errors="replace").read().strip()
                h = hashlib.sha1(t.encode()).hexdigest()
                texts[h] = t
                used_by.setdefault(h, []).append(f"bundled font notice: {f}")

out = [
    "Flint — third-party software notices",
    "",
    "Flint itself is licensed under MIT OR Apache-2.0. It includes the open-source components below,",
    "which remain under their own licenses. Their license texts follow the list.",
    "",
    *rows,
    "",
]
for h, t in texts.items():
    out += ["=" * 78, "Used by: " + ", ".join(sorted(set(used_by[h]))), "=" * 78, "", t, ""]
open(os.path.join(root, "THIRD_PARTY_LICENSES.txt"), "w").write("\n".join(out))
print(f"{len(rows)} crates, {len(texts)} distinct license texts", file=sys.stderr)
