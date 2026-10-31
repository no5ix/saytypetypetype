#!/usr/bin/env python3
"""Report translation keys used via t("...") that have no English entry.

Run from the repository root: `python3 scripts/i18n-audit.py`.
Keys are the Simplified Chinese source strings; entries live in
`web/src/i18n/parts/*.ts`.
"""

import json
import os
import re
import sys

SOURCE_DIR = "web/src"
PART_DIR = "web/src/i18n/parts"
PARTS = ["common", "home", "settings", "shortcuts", "models"]


def unescape(text: str) -> str:
    return text.replace(r"\n", "\n").replace(r"\"", '"')


def used_keys():
    keys = {}
    for dirpath, _dirnames, filenames in os.walk(SOURCE_DIR):
        if os.sep + "i18n" in dirpath:
            continue
        for name in filenames:
            if not name.endswith((".ts", ".tsx")) or ".test." in name:
                continue
            path = os.path.join(dirpath, name)
            src = open(path, encoding="utf-8").read()
            for pattern in (r'\bt\(\s*"((?:[^"\\]|\\.)*)"', r"\bt\(\s*`((?:[^`$\\]|\\.)*)`"):
                for match in re.finditer(pattern, src):
                    keys.setdefault(unescape(match.group(1)), set()).add(path)
            # Dynamic keys: t(model.name), t(meta.label), t(section.title), …
            for match in re.finditer(r"\bt\(([A-Za-z_][\w.]*)\)", src):
                keys.setdefault(f"dynamic:{match.group(1)}", set()).add(path)
    return keys


def entries():
    table = {}
    for part in PARTS:
        src = open(os.path.join(PART_DIR, f"{part}.ts"), encoding="utf-8").read()
        for match in re.finditer(
            r'^\s*(?:"((?:[^"\\]|\\.)*)"|([^\s":]+))\s*:\s*"((?:[^"\\]|\\.)*)",',
            src,
            re.M,
        ):
            key = match.group(1) if match.group(1) is not None else match.group(2)
            table[unescape(key)] = (unescape(match.group(3)), part)
    return table


def main() -> int:
    keys = used_keys()
    table = entries()
    static = {k: v for k, v in keys.items() if not k.startswith("dynamic:")}
    missing = {k: v for k, v in static.items() if k not in table}
    unused = {k: v for k, v in table.items() if k not in static}
    print(f"== MISSING {len(missing)}")
    for key in sorted(missing):
        print(json.dumps(key, ensure_ascii=False))
    print(f"== UNUSED {len(unused)}")
    for key in sorted(unused):
        print(json.dumps(key, ensure_ascii=False), table[key][0])
    print(f"== DYNAMIC {len(keys) - len(static)}")
    for key in sorted(k for k in keys if k.startswith("dynamic:")):
        print(key)
    return 0


if __name__ == "__main__":
    sys.exit(main())
