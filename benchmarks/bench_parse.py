"""Benchmark: pure-Python text pipeline vs the lue_rs Rust extension.

Generates a synthetic Chinese web-novel TXT (~3 MB, line-per-paragraph
layout with titled chapters) and measures the hot paths that dominate
book-open time. Run from the repo root:

    python benchmarks/bench_parse.py
"""

import io
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from rich.console import Console

from lue import _rust
from lue import content_parser as cp

DIGITS = "零一二三四五六七八九十"
SAMPLE_SIZE = 500 * 1024


def build_big_book(chapters=1200, body_lines=90):
    rng = __import__("random").Random(1234)
    verbs = ["说道", "看著", "想起", "离开", "转身", "点头", "沉默", "笑了"]
    parts = ["书名:基准测试\n\n"]
    for i in range(1, chapters + 1):
        parts.append(f"第{DIGITS[i % 10]}章 试验剧情之{i}\n\n")
        for _ in range(body_lines):
            verb = rng.choice(verbs)
            parts.append(f"他{verb}一句话,场面顿时安静下来。有人低声议论!\n")
        parts.append("\n")
    return "".join(parts)


def bench(fn, repeat=3):
    best = float("inf")
    result = None
    for _ in range(repeat):
        t0 = time.perf_counter()
        result = fn()
        best = min(best, time.perf_counter() - t0)
    return best, result


def fmt(seconds):
    return f"{seconds * 1000:8.1f} ms"


def main():
    if _rust.lue_rs is None:
        sys.exit("lue_rs extension not built; run maturin first")

    book = build_big_book()
    lines = [l for l in book.split("\n") if l.strip()]
    paragraphs = [l for l in lines if not l.startswith("第")]
    sample = book[:SAMPLE_SIZE]
    path = Path(__file__).resolve().parent / "bench_book.txt"
    path.write_text(book, encoding="utf-8")
    console = Console(file=io.StringIO(), width=80)

    print(f"book: {len(book) / 1e6:.1f} MB, {len(lines)} lines, {len(paragraphs)} paragraphs\n")

    tasks = []

    def py_clean():
        return [cp._clean_visual_text_py(l.strip()) for l in lines]

    def rs_clean():
        return [cp.clean_visual_text(l.strip()) for l in lines]

    def py_split():
        return [cp._split_into_sentences_py(p) for p in paragraphs]

    def rs_split():
        return [_rust.lue_rs.split_sentences(p) for p in paragraphs]

    def py_toc():
        _rust.lue_rs, saved = None, _rust.lue_rs
        try:
            return cp._pick_toc_rule(sample)
        finally:
            _rust.lue_rs = saved

    def rs_toc():
        return cp._pick_toc_rule(sample)

    def py_extract():
        _rust.lue_rs, saved = None, _rust.lue_rs
        try:
            return cp._extract_content_txt(str(path), console)
        finally:
            _rust.lue_rs = saved

    def rs_extract():
        return cp._extract_content_txt(str(path), console)

    tasks.append(("clean_visual_text (all lines)", py_clean, rs_clean))
    tasks.append(("split_into_sentences (all paras)", py_split, rs_split))
    tasks.append(("_pick_toc_rule (512K sample)", py_toc, rs_toc))
    tasks.append(("_extract_content_txt (full pipeline)", py_extract, rs_extract))

    print(f"{'task':38} {'Python':>12} {'Rust':>12} {'speedup':>9}")
    print("-" * 75)
    for name, py_fn, rs_fn in tasks:
        py_t, py_res = bench(py_fn)
        rs_t, rs_res = bench(rs_fn)
        match = " (equal)" if py_res == rs_res else " (DIFFERS!)"
        print(f"{name:38} {fmt(py_t):>12} {fmt(rs_t):>12} {py_t / rs_t:8.1f}x{match}")

    path.unlink()


if __name__ == "__main__":
    main()
