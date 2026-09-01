"""Verify the ellipsis hypothesis: does the precheck miss U+2026?"""

import sys

sys.path.insert(0, "/mnt/e/GitHub/lue")

from lue.content_parser import _clean_visual_text_py

# Sample from the real book file, read exactly as lue reads it.
book = "/home/junerver/books/高考.txt"
raw = open(book, "rb").read()
for enc in ("utf-8-sig", "gb18030"):
    try:
        content = raw.decode(enc)
        break
    except UnicodeDecodeError:
        continue
lines = [l for l in content.split("\n") if l.strip()]

UNICODE_SRC = set("×÷±≤≥≠≈∞%+=°™®©§’\u200b\u200c\u200d\ufeff\u00ad")
REMOVE = set("-=.~`^")
allsp = UNICODE_SRC | REMOVE | set("*") | set("._[]{#!?,:;`~")


def precheck_safe(line):
    if line != line.strip():
        return False
    prev_ws = False
    for ch in line:
        if ch in allsp:
            return False
        if ch.isspace():
            if ch != " " or prev_ws:
                return False
            prev_ws = True
        else:
            prev_ws = False
    return True


# 1. What is the actual encoding? count how many lines contain U+2026
ellipsis_lines = sum(1 for l in lines if "…" in l)
print(f"lines with ellipsis U+2026: {ellipsis_lines}")

# 2. Show a real safe-diff line with codepoints
count = 0
for l in lines:
    stripped = l.strip()
    cleaned = _clean_visual_text_py(stripped)
    if precheck_safe(stripped) and cleaned != stripped:
        diff_chars = [c for c in stripped if c not in cleaned]
        print(f"line chars: {len(stripped)}, cleaned: {len(cleaned)}")
        print(f"changed chars: {[hex(ord(c)) for c in diff_chars][:10]}")
        # show if the change is ellipsis collapse
        if "…" in stripped:
            print("  contains U+2026 ellipsis")
        count += 1
        if count >= 2:
            break
print("shown:", count)