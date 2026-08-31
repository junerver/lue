"""Benchmark the document-layout pipeline (update_document_layout's hot loops).

Measures where time goes when opening a large book: rich Text.wrap per
paragraph, sentence splitting (per-call vs batch), and the two-pointer
sentence->line mapping. Run from repo root:

    python benchmarks/bench_layout.py
"""

import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from rich.text import Text

from lue import _rust
from lue import content_parser as cp


def bench(fn, repeat=3):
    best = float("inf")
    result = None
    for _ in range(repeat):
        t0 = time.perf_counter()
        result = fn()
        best = min(best, time.perf_counter() - t0)
    return best, result


def main():
    rng = __import__("random").Random(42)
    verbs = ["说道", "看著", "想起", "离开", "转身", "点头", "沉默", "笑了"]
    chapters = 1200
    paragraphs = []
    for _ in range(chapters):
        for _ in range(60):
            verb = rng.choice(verbs)
            paragraphs.append(f"他{verb}一句话,场面顿时安静下来。有人低声议论!\n")
    print(f"paragraphs: {len(paragraphs)}")

    width = 100
    text_objs = [Text(p, justify="left", no_wrap=False) for p in paragraphs]

    class FakeConsole:
        color_system = None

        def __init__(self):
            self.width = width

    console = FakeConsole()

    def py_wrap():
        return [t.wrap(console, width) for t in text_objs]

    wrap_t, _ = bench(py_wrap, repeat=1)
    print(f"rich Text.wrap (all paragraphs):  {wrap_t * 1000:8.1f} ms")

    if _rust.lue_rs is not None:
        def rs_wrap():
            return [_rust.lue_rs.wrap_paragraph(p, width) for p in paragraphs]

        rs_t, _ = bench(rs_wrap)
        print(f"rust wrap_paragraph (all paras):  {rs_t * 1000:8.1f} ms   ({wrap_t / rs_t:.1f}x)")

        def py_wrap_plain():
            return [[line.plain for line in t.wrap(console, width)] for t in text_objs]

        def rs_wrap_check():
            return [_rust.lue_rs.wrap_paragraph(p, width) for p in paragraphs]

        _, pyr = bench(py_wrap_plain, repeat=1)
        _, rsr = bench(rs_wrap_check)
        if pyr != rsr:
            for i, (a, b) in enumerate(zip(pyr, rsr)):
                if a != b:
                    print(f"first diff at paragraph {i}: {paragraphs[i]!r}")
                    for j, (x, y) in enumerate(zip(a, b)):
                        if x != y:
                            print(f"  line {j} RICH: {x!r}")
                            print(f"  line {j} RUST: {y!r}")
                            break
                    break
            print("equal outputs: False")
        else:
            print("equal outputs: True")

    def py_split_each():
        return [cp.split_into_sentences(p) for p in paragraphs]

    def rs_split_batch():
        if _rust.lue_rs is None:
            return None
        return _rust.lue_rs.split_sentences_batch(paragraphs)

    if _rust.lue_rs is not None:
        per_t, _ = bench(py_split_each)
        batch_t, _ = bench(rs_split_batch)
        print(f"split_into_sentences per-call:    {per_t * 1000:8.1f} ms")
        print(f"split_into_sentences batch:       {batch_t * 1000:8.1f} ms")

    # Two-pointer sentence->line mapping, as in update_document_layout.
    sentences = [cp.split_into_sentences(p) for p in paragraphs]
    wrapped = py_wrap()
    assert len(sentences) == len(wrapped)

    def py_mapping():
        out = {}
        for para_idx, (sent_list, lines) in enumerate(zip(sentences, wrapped)):
            line_idx = 0
            line_offset = 0
            current_char_pos = 0
            plain_lines = [line.plain for line in lines]
            for sent_idx, sentence in enumerate(sent_list):
                sentence_start = current_char_pos
                while (
                    line_idx < len(plain_lines)
                    and sentence_start >= line_offset + len(plain_lines[line_idx])
                ):
                    line_offset += len(plain_lines[line_idx])
                    line_idx += 1
                if line_idx < len(plain_lines):
                    out[(para_idx, sent_idx)] = line_idx
                current_char_pos = sentence_start + len(sentence) + 1
        return out

    map_t, _ = bench(py_mapping)
    print(f"two-pointer sentence->line map:    {map_t * 1000:8.1f} ms")


if __name__ == "__main__":
    main()
