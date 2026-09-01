"""Same-process A/B: old layout path (python split + wrap + two-pointer)
vs new layout_paragraph (all-Rust). Eliminates cross-run noise.
"""

import io
import os
import shutil
import sys
import time

from lue import content_parser as cp, _rust
from rich.console import Console
from rich.text import Text

book = "/home/junerver/books/高考.txt"
tmp = "/tmp/gk_ab.txt"
shutil.copy(book, tmp)
console = Console(file=io.StringIO(), width=80)
WIDTH = 100

chapters = cp._extract_content_txt(tmp, console)


def old_layout(chapters):
    document_lines = []
    position_to_line = {}
    line_to_position = {}
    paragraph_line_ranges = {}
    for chap_idx, chapter in enumerate(chapters):
        if chap_idx > 0:
            document_lines.append("")
        for para_idx, paragraph in enumerate(chapter):
            pstart = len(document_lines)
            plain_text = Text(paragraph, justify="left", no_wrap=False)
            wrapped = [l.plain for l in plain_text.wrap(console, WIDTH)]
            sentences = cp.split_into_sentences(paragraph)
            li = 0
            lo = 0
            cur = 0
            for si, s in enumerate(sentences):
                ss = cur
                while li < len(wrapped) and ss >= lo + len(wrapped[li]):
                    lo += len(wrapped[li])
                    li += 1
                if li < len(wrapped):
                    position_to_line[(chap_idx, para_idx, si)] = pstart + li
                cur = ss + len(s) + 1
            for k in range(len(wrapped)):
                line_to_position[pstart + k] = (chap_idx, para_idx, 0)
            document_lines.extend(wrapped)
            if para_idx < len(chapter) - 1:
                document_lines.append("")
    return document_lines, position_to_line, line_to_position


def new_layout(chapters):
    document_lines = []
    position_to_line = {}
    line_to_position = {}
    for chap_idx, chapter in enumerate(chapters):
        if chap_idx > 0:
            document_lines.append("")
        for para_idx, paragraph in enumerate(chapter):
            pstart = len(document_lines)
            wrapped, sent_lines, _ = _rust.lue_rs.layout_paragraph(paragraph, WIDTH)
            for si, li in enumerate(sent_lines):
                if li >= 0:
                    position_to_line[(chap_idx, para_idx, si)] = pstart + li
            for k in range(len(wrapped)):
                line_to_position[pstart + k] = (chap_idx, para_idx, 0)
            document_lines.extend(wrapped)
            if para_idx < len(chapter) - 1:
                document_lines.append("")
    return document_lines, position_to_line, line_to_position


# warmup
new_layout(chapters)

for label, fn in (("old(py split+wrap+2ptr)", old_layout), ("new(layout_paragraph)", new_layout)):
    for run in range(3):
        t0 = time.perf_counter()
        dl, p2l, l2p = fn(chapters)
        dt = (time.perf_counter() - t0) * 1000
        print(f"{label} run{run+1}: {dt:.0f} ms (lines {len(dl)}, pos {len(p2l)})")

os.unlink(tmp)