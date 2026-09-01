"""Characterization + parity tests for the text pipeline.

Two layers:

1. Characterization cases pin the observable behaviour of the public
   ``content_parser`` API regardless of backend. They run with the Rust
   extension when it is importable and with the pure-Python fallback
   otherwise (and the fallback is forced in dedicated tests).

2. Parity cases compare the Rust extension against the retained
   pure-Python implementations function-by-function, including a seeded
   random-text fuzz over the mixed CJK/Latin punctuation space. They are
   skipped when ``lue_rs`` is not built.
"""

import io
import random

import pytest
from rich.console import Console

from lue import _rust
from lue import content_parser as cp


def make_console():
    return Console(file=io.StringIO(), width=80)


@pytest.fixture
def force_py(monkeypatch):
    monkeypatch.setattr(_rust, "lue_rs", None)


@pytest.fixture
def force_rust(monkeypatch):
    if _rust.lue_rs is None:
        pytest.skip("lue_rs extension not built")
    monkeypatch.setattr(_rust, "lue_rs", _rust.lue_rs)


def build_book(chapters=80, body_lines=300):
    # One short paragraph per line (typical Chinese web-novel layout): keeps
    # cjk_avg_len below _AVG_LINE_AS_PARAGRAPH_LIMIT so the line-as-paragraph
    # branch runs and title-rule chaptering stays enabled.
    digits = "零一二三四五六七八九十"
    parts = ["测试书名\n\n"]
    for i in range(1, chapters + 1):
        parts.append(f"第{digits[i % 10]}章 试验{i}\n\n")
        parts.append("这里的正文一行一句比较短。\n" * body_lines)
        parts.append("\n")
    return "".join(parts)


# --------------------------------------------------------------------------
# Sentence splitting — characterization (values verified against the
# pure-Python implementation on this branch).
# --------------------------------------------------------------------------

SENTENCE_CASES = [
    ("", [""]),
    ("hello world", ["hello world"]),
    ("One. Two. Three.", ["One.", "Two.", "Three."]),
    ("End. ", ["End."]),
    ("Mr. Smith arrived. He left.", ["Mr. Smith arrived.", "He left."]),
    ("J. F. Kennedy spoke.", ["J. F. Kennedy spoke."]),
    ("etc. and more. End.", ["etc. and more.", "End."]),
    ("3.14 is pi. Yes.", ["3.14 is pi.", "Yes."]),
    ("What?! Really?!", ["What?!", "Really?!"]),
    ("今天天气很好。我们去公园吧!", ["今天天气很好。", "我们去公园吧!"]),
    ("。。哈哈", ["。。", "哈哈"]),
    # Half-width !/? only split when followed by whitespace.
    ("半。角!混?合。", ["半。", "角!混?合。"]),
    # Adjacent zero-width cuts across a quote run.
    ("。””汉", ["。”", "”", "汉"]),
    ("他说:“好。”然后走了。", ["他说:“好。”", "然后走了。"]),
    ("नमस्ते। दुनिया", ["नमस्ते।", "दुनिया"]),
]


@pytest.mark.parametrize("text,expected", SENTENCE_CASES)
def test_split_sentences(text, expected):
    assert cp.split_into_sentences(text) == expected


@pytest.mark.parametrize("text,expected", SENTENCE_CASES)
def test_split_sentences_fallback(force_py, text, expected):
    assert cp.split_into_sentences(text) == expected


def test_split_sentences_cached_identity():
    # lru_cache: same input returns the same object, callers treat it read-only.
    assert cp.split_into_sentences("abc。def") is cp.split_into_sentences("abc。def")


# --------------------------------------------------------------------------
# clean_visual_text — characterization
# --------------------------------------------------------------------------

CLEAN_CASES = [
    ("", ""),
    ("plain text", "plain text"),
    ("__CODE_BLOCK__    x = 1", "    x = 1"),
    ("a . . . b", "a... b"),
    ("a . . b", "a..b"),
    ("a....b", "a... b"),
    ("----", ""),
    ("====", ""),
    ("****", ""),
    ("###", "###"),
    ("#### Header", "Header"),
    ("# Header", "Header"),
    ("**bold** text", "bold text"),
    ("*em*", "em"),
    ("__underline__", "underline"),
    ("_ital_", "ital"),
    ("`code`", "code"),
    ("~~gone~~", "gone"),
    ("[label](http://x)", "label"),
    ("[text][ref]", "text"),
    ("[ref]: http://x", ""),
    ("a , b", "a, b"),
    ("a!!b", "a! !b"),
    ("3 × 4", "3 multiplied by 4"),
    ("50%", "50 percent"),
    ("don’t", "don't"),
    ("…", "..."),
    ("x……y", "x... y"),
    ("a\u200bb", "ab"),
    ("x\u3000\u3000y", "x y"),  # full-width spaces collapse
]


@pytest.mark.parametrize("text,expected", CLEAN_CASES)
def test_clean_visual_text(text, expected):
    assert cp.clean_visual_text(text) == expected


@pytest.mark.parametrize("text,expected", CLEAN_CASES)
def test_clean_visual_text_fallback(force_py, text, expected):
    assert cp.clean_visual_text(text) == expected


def test_clean_visual_text_passthrough():
    assert cp.clean_visual_text(None) is None
    assert cp.clean_visual_text("") == ""


# --------------------------------------------------------------------------
# TXT chapter rules — characterization
# --------------------------------------------------------------------------


def test_pick_toc_rule_prefers_blank_prefixed_rule():
    # Titles are preceded by "\n", so rule 0's (?<=[　\s]) lookbehind holds and
    # its 80 valid chapters trigger the early exit before rule 1 is scored.
    book = build_book()
    assert cp._pick_toc_rule(book) == cp.TXT_TOC_RULES[0]


def test_collect_title_lines():
    book = build_book()
    lines = cp._collect_title_lines(book, cp.TXT_TOC_RULES[0])
    assert len(lines) == 80
    # line 2 = first title (0-based); each chapter spans 303 lines.
    assert sorted(lines)[:3] == [2, 305, 608]


def extract_doc_lines(content):
    doc_lines = []
    for i, line in enumerate(content.split("\n")):
        text = cp.clean_visual_text(line.strip())
        if text:
            doc_lines.append((i, text))
    return doc_lines


def test_split_txt_by_rule_structure():
    book = build_book()
    rule = cp._pick_toc_rule(book)
    chapters = cp._split_txt_by_rule(book, extract_doc_lines(book), rule)
    # intro (title line) + 80 titled chapters
    assert len(chapters) == 81
    assert chapters[0] == ["测试书名"]
    assert chapters[1][0].startswith("第一章")
    assert all(len(ch) >= 1 for ch in chapters)


def test_split_txt_by_size():
    lines = ["段落"] * 100
    chapters = cp._split_txt_by_size(lines, max_chars=30)
    assert len(chapters) > 1
    assert sum(len(c) for c in chapters) == 100


def test_split_long_text_into_paragraphs():
    text = "句子。" * 100
    paras = cp._split_long_text_into_paragraphs(text)
    assert len(paras) >= 2
    assert all(len(p) >= 200 for p in paras[:-1])
    assert "".join(paras) == text


# --------------------------------------------------------------------------
# Full TXT extraction pipeline — characterization
# --------------------------------------------------------------------------


def write_txt(tmp_path, content, encoding="utf-8"):
    path = tmp_path / "book.txt"
    path.write_bytes(content.encode(encoding))
    return str(path)


def test_extract_content_txt_chapters(tmp_path):
    path = write_txt(tmp_path, build_book(chapters=10))
    chapters = cp._extract_content_txt(path, make_console())
    assert len(chapters) == 11  # intro + 10 chapters
    assert chapters[0] == ["测试书名"]
    assert chapters[1][0].startswith("第一章")


def test_extract_content_txt_gb18030(tmp_path):
    path = write_txt(tmp_path, build_book(chapters=3), encoding="gb18030")
    chapters = cp._extract_content_txt(path, make_console())
    assert len(chapters) == 4
    assert chapters[1][0].startswith("第一章")


def test_extract_content_txt_fallback_matches(tmp_path):
    path = write_txt(tmp_path, build_book(chapters=8))
    with_rust = cp._extract_content_txt(path, make_console())
    with_py = None

    original = _rust.lue_rs
    _rust.lue_rs = None
    try:
        with_py = cp._extract_content_txt(path, make_console())
    finally:
        _rust.lue_rs = original

    assert with_rust == with_py


def test_extract_content_txt_single_line(tmp_path):
    # Almost no newlines: sentence-splitting path, size-based chaptering.
    content = "这是一句很长的话。再加一句!" * 500
    path = write_txt(tmp_path, content)
    chapters = cp._extract_content_txt(path, make_console())
    assert chapters
    assert all(ch for ch in chapters)


def test_extract_content_txt_blank_line_paragraphs(tmp_path):
    # English-style layout: paragraphs separated by blank lines get merged.
    content = "First paragraph line one\ncontinues here.\n\nSecond paragraph.\n\nThird."
    path = write_txt(tmp_path, content)
    chapters = cp._extract_content_txt(path, make_console())
    assert chapters[0] == [
        "First paragraph line one continues here.",
        "Second paragraph.",
        "Third.",
    ]


# --------------------------------------------------------------------------
# Parity: Rust extension vs pure-Python fallbacks (skipped without lue_rs)
# --------------------------------------------------------------------------

FUZZ_POOL = list(
    "abcdefgABCDEFG xyz.!?。！？“”‘’…、,;\n\t0123456789"
    "章第一二三四五六七八九十卷节集部篇正文序章楔子后记尾声番外"
    "MrDrvs等co【】[]()----****####___"
)


def random_texts(count=400, seed=20260831):
    rng = random.Random(seed)
    texts = [""]
    for _ in range(count):
        length = rng.randint(0, 160)
        texts.append("".join(rng.choice(FUZZ_POOL) for _ in range(length)))
    return texts


@pytest.mark.skipif(_rust.lue_rs is None, reason="lue_rs extension not built")
def test_parity_split_sentences_fuzz():
    for text in random_texts():
        assert _rust.lue_rs.split_sentences(text) == cp._split_into_sentences_py(text), repr(text)


@pytest.mark.skipif(_rust.lue_rs is None, reason="lue_rs extension not built")
def test_parity_clean_visual_text_fuzz():
    for text in random_texts(seed=777):
        assert _rust.lue_rs.clean_visual_text(text) == cp._clean_visual_text_py(text), repr(text)


@pytest.mark.skipif(_rust.lue_rs is None, reason="lue_rs extension not built")
def test_parity_clean_txt_doc_lines():
    for text in random_texts(count=50, seed=31):
        content = "\n".join(random_texts(count=20, seed=hash(text) & 0xFFFF))
        expected = []
        for i, line in enumerate(content.split("\n")):
            cleaned = cp.clean_visual_text(line.strip())
            if cleaned:
                expected.append((i, cleaned))
        assert _rust.lue_rs.clean_txt_doc_lines(content) == expected


@pytest.mark.skipif(_rust.lue_rs is None, reason="lue_rs extension not built")
def test_parity_clean_txt_doc_lines_ellipsis_and_wide_space():
    # Lines that trigger the non-fast-path cleaning: ellipsis collapse,
    # full-width space normalization, markdown, unicode replacements.
    content = (
        "普通的一行没有特殊字符\n"
        "这一行有省略号……还有……\n"
        "　　这行开头是全角空格\n"
        "**加粗**和`代码`\n"
        "50%和3×4\n"
        "a . . . b\n"
        "纯文本末尾\n"
    )
    expected = []
    for i, line in enumerate(content.split("\n")):
        cleaned = cp.clean_visual_text(line.strip())
        if cleaned:
            expected.append((i, cleaned))
    assert _rust.lue_rs.clean_txt_doc_lines(content) == expected


@pytest.mark.skipif(_rust.lue_rs is None, reason="lue_rs extension not built")
def test_parity_pick_toc_rule_and_lines():
    books = [
        build_book(chapters=10),
        build_book(chapters=200, body_lines=250),
        "无标题的正文。\n" * 5000,
        "".join(f"Chapter {i} Name\n\n" + "Body text here. " * 80 + "\n\n" for i in range(1, 90)),
        "".join(f"{i}、标题名\n\n" + "正文。" * 400 + "\n\n" for i in range(1, 90)),
    ]
    for book in books:
        rust_idx = _rust.lue_rs.pick_toc_rule(book)
        _rust.lue_rs, original = None, _rust.lue_rs
        try:
            py_rule = cp._pick_toc_rule(book)
        finally:
            _rust.lue_rs = original
        if rust_idx is None:
            assert py_rule is None
            continue
        assert cp.TXT_TOC_RULES[rust_idx] == py_rule
        rust_lines = set(_rust.lue_rs.collect_title_lines(book, rust_idx))
        assert rust_lines == cp._collect_title_lines(book, py_rule)


@pytest.mark.skipif(_rust.lue_rs is None, reason="lue_rs extension not built")
def test_parity_split_long_text_paragraphs_fuzz():
    rng = random.Random(5)
    for _ in range(50):
        text = "".join(rng.choice("。！？!?；;ab汉字 ") for _ in range(rng.randint(0, 500)))
        assert _rust.lue_rs.split_long_text_paragraphs(text, 200) == (
            cp._split_long_text_into_paragraphs(text)
        )


# --------------------------------------------------------------------------
# Word wrap parity: lue_rs.wrap_paragraph vs rich Text.wrap (ui.py:195)
# --------------------------------------------------------------------------

WRAP_POOL = (
    "abcdefghijklmnop "  # 16 latin letters + space
    "你好世界这是中文句子。 "
    "The quick brown fox jumps over the lazy dog. "
    "0123456789"
    "……—"
    ",\"'?!;:"
    "　"  # full-width space
    "\t"
)


def rich_wrap_lines(text, width):
    from rich.text import Text

    class FakeConsole:
        color_system = None

        def __init__(self):
            self.width = width

    console = FakeConsole()
    return [line.plain for line in Text(text, justify="left", no_wrap=False).wrap(console, width)]


def random_paragraphs(count=300, seed=99):
    rng = random.Random(seed)
    paras = []
    for _ in range(count):
        length = rng.randint(0, 300)
        paras.append("".join(rng.choice(WRAP_POOL) for _ in range(length)))
    return paras


@pytest.mark.skipif(_rust.lue_rs is None, reason="lue_rs extension not built")
@pytest.mark.parametrize("width", [20, 40, 80])
def test_parity_wrap_manual_cases(width):
    cases = [
        "",
        "Hello world how are you today",
        "你好世界这是一段中文文本没有任何空格",
        "a b c",
        "supercalifragilisticexpialidocious longword",
        "Some text with  extra  spaces   inside",
        "x" * 200,
        "（这是一段带全角括号的中文）然后继续",
        "mixed 中文 and ASCII words\nwith a newline",
        "Ends with space ",
    ]
    for text in cases:
        assert _rust.lue_rs.wrap_paragraph(text, width) == rich_wrap_lines(text, width), (
            repr(text),
            width,
        )


@pytest.mark.skipif(_rust.lue_rs is None, reason="lue_rs extension not built")
@pytest.mark.parametrize("width", [30, 70, 100])
def test_parity_wrap_fuzz(width):
    for text in random_paragraphs(seed=width):
        assert _rust.lue_rs.wrap_paragraph(text, width) == rich_wrap_lines(text, width), (
            repr(text),
            width,
        )


# --------------------------------------------------------------------------
# Layout parity: update_document_layout produces identical document_lines
# with the Rust wrap and with the rich fallback.
# --------------------------------------------------------------------------


class _StubReader:
    """Just enough state for ui.update_document_layout."""

    def __init__(self, chapters, console):
        self.chapters = chapters
        self.console = console
        self._layout_cache = None
        self._wrap_cache = None
        self.document_lines = []
        self.line_to_position = {}
        self.position_to_line = {}
        self.paragraph_line_ranges = {}
        # update_document_layout's tail touches these only when set
        self._initial_load_complete = False


@pytest.mark.skipif(_rust.lue_rs is None, reason="lue_rs extension not built")
def test_parity_update_document_layout(tmp_path, monkeypatch):
    from lue import ui

    book = build_book(chapters=6)
    chapters = [
        [t for t in chapter] for chapter in cp._extract_content_txt(
            str(write_txt(tmp_path, book)), make_console()
        )
    ]

    results = {}
    for backend in ("rust", "rich"):
        reader = _StubReader([list(c) for c in chapters], make_console())
        monkeypatch.setattr(_rust, "lue_rs", _rust.lue_rs if backend == "rust" else None)
        monkeypatch.setattr(ui.config, "UI_MODE", 0)
        monkeypatch.setattr(ui, "get_terminal_size", lambda: (100, 40))
        ui.update_document_layout(reader)
        # document_lines now stores plain strings (Text reconstructed on render)
        results[backend] = [
            line.plain if hasattr(line, "plain") else line for line in reader.document_lines
        ]
        results[backend + "_map"] = dict(reader.position_to_line)

    assert results["rust"] == results["rich"]
    assert results["rust_map"] == results["rich_map"]
