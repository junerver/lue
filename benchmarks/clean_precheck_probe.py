"""Enumerate which characters/patterns clean_visual_text can change, to build
a fast-path precheck: if a line contains none of these, cleaning is a no-op.
"""

from lue.content_parser import _clean_visual_text_py

# All chars that appear as replace targets or regex-significant chars.
# Build from the actual rules in the Python implementation.
import re

# 1. spaced-dot / ellipsis rules: '.' and whitespace runs
# 2. unicode replacements: the source chars
UNICODE_SRC = "×÷±≤≥≠≈∞%+=°™®©§’\u200b\u200c\u200d\ufeff\u00ad"
# 3. markdown markers: * _ ` ~ [ ] # and links
# 4. punctuation-adjacent: , ! ? ; :
REMOVE_RUNS = set("-=~`^")  # [-_=~`^]{3,}
STAR_RUNS = {"*"}
HASH_RUNS = {"#"}
PLUS_RUNS = {"+"}
PIPE_RUNS = {"|"}
BACKSLASH_RUNS = {"\\"}
SLASH_RUNS = {"/"}

all_special = set(UNICODE_SRC) | REMOVE_RUNS | STAR_RUNS | HASH_RUNS | PLUS_RUNS | PIPE_RUNS | BACKSLASH_RUNS | SLASH_RUNS
all_special |= set(".*_`~[]#!?,:;")
all_special |= {"\u2026"}  # ellipsis collapses to ...

print("special chars:", "".join(sorted(all_special)))
print("as codepoints:", [hex(ord(c)) for c in sorted(all_special)])


def precheck_safe(line: str) -> bool:
    """Return True if clean_visual_text(line) == line (modulo trim)."""
    # conservative: any special char, non-ASCII whitespace (e.g. \u3000,
    # which \s+ collapses to ' '), or whitespace run -> not safe
    if line != line.strip():
        return False
    if not line:
        return True
    prev_ws = False
    for ch in line:
        if ch in all_special:
            return False
        if ch.isspace():
            if ch != " ":  # any non-ASCII-space whitespace gets normalized
                return False
            if prev_ws:
                return False  # internal whitespace collapse
            prev_ws = True
        else:
            prev_ws = False
    return True


# Validate against real book lines
import sys

book = sys.argv[1] if len(sys.argv) > 1 else None
if not book:
    book = "/home/junerver/books/高考.txt"
content = open(book, encoding="utf-8").read().replace("\r\n", "\n")
lines = [l for l in content.split("\n") if l.strip()]
print(f"\nbook: {book}, non-empty lines: {len(lines)}")

safe_ok = 0
safe_false_pos = 0  # precheck says safe but clean changes -> BUG
unsafe_ok = 0
unsafe_same = 0  # precheck says unsafe but clean keeps same (missed fast path)
for l in lines:
    stripped = l.strip()
    cleaned = _clean_visual_text_py(stripped)
    safe = precheck_safe(stripped)
    if safe:
        if cleaned == stripped:
            safe_ok += 1
        else:
            safe_false_pos += 1
    else:
        if cleaned == stripped:
            unsafe_same += 1
        else:
            unsafe_ok += 1

print(f"precheck-safe & clean==line:  {safe_ok} ({safe_ok/len(lines)*100:.1f}%)")
print(f"precheck-safe but clean diff: {safe_false_pos}  <-- must be 0")
print(f"precheck-unsafe & clean diff: {unsafe_ok}")
print(f"precheck-unsafe & clean same: {unsafe_same} (missed fast path)")
assert safe_false_pos == 0, "precheck false-positive: would corrupt lines!"