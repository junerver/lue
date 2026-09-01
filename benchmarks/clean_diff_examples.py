"""Show concrete lines where precheck says safe but clean changes them."""

import sys

sys.path.insert(0, "/mnt/e/GitHub/lue")

from lue.content_parser import _clean_visual_text_py

book = "/home/junerver/books/高考.txt"
content = open(book, encoding="utf-8").read().replace("\r\n", "\n")
lines = [l for l in content.split("\n") if l.strip()]

UNICODE_SRC = set("×÷±≤≥≠≈∞%+=°™®©§’\u200b\u200c\u200d\ufeff\u00ad")
REMOVE = set("-=.~`^")  # the chars in [-_=~`^]{3,}
STAR = "*"
allsp = UNICODE_SRC | REMOVE | set("*") | set("._[]{#!?,:;`~")  # wider net


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


count = 0
for l in lines:
    stripped = l.strip()
    cleaned = _clean_visual_text_py(stripped)
    if precheck_safe(stripped) and cleaned != stripped:
        print("SAFE-DIFF line:", repr(stripped[:80]))
        print("  cleaned:", repr(cleaned[:80]))
        count += 1
        if count >= 6:
            break
print("total shown:", count)