#!/usr/bin/env python3
"""Quote object keys in the i18n part files that are not valid JS identifiers.

Chinese characters are valid identifier characters, but keys containing spaces,
punctuation, braces or digits at the start must be quoted. Comment lines and
declarations are left alone. Run from the repo root after editing entries by
hand: `python3 scripts/i18n-quote-keys.py`.
"""

import re
import sys

PART_DIR = "web/src/i18n/parts"
PARTS = ["common", "home", "settings", "shortcuts", "models"]

IDENT = re.compile(
    r"^[A-Za-z_$\u4e00-\u9fff\u3400-\u4dbf][A-Za-z0-9_$\u4e00-\u9fff\u3400-\u4dbf]*$"
)
# An entry line is `key: "value",` — the value may wrap, so only require the
# opening quote. Comments, blank lines and declarations never match.
ENTRY = re.compile(r"^(\s*)(\"(?:[^\"\\]|\\.)*\"|[^\"\s][^:]*?)(\s*):\s*(\".*)$")
SKIP = re.compile(r"^\s*(/?\*|//|export|const|import|type|\}|\{)")


def main() -> int:
    changed = 0
    for part in PARTS:
        path = f"{PART_DIR}/{part}.ts"
        out = []
        for line in open(path, encoding="utf-8").read().split("\n"):
            if SKIP.match(line):
                out.append(line)
                continue
            match = ENTRY.match(line)
            if match:
                indent, key, colon, rest = match.groups()
                bare = key.strip('"')
                if not IDENT.match(bare):
                    escaped = bare.replace("\\", "\\\\").replace('"', '\\"')
                    line = f'{indent}"{escaped}"{colon}{rest}'
                    changed += 1
            out.append(line)
        open(path, "w", encoding="utf-8").write("\n".join(out))
    print(f"quoted {changed} keys")
    return 0


if __name__ == "__main__":
    sys.exit(main())
