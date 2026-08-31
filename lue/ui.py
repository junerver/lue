import asyncio
import os
import sys
import platform
import re
from rich.console import Console
from rich.text import Text
from rich.panel import Panel
from rich.table import Table
from rich.align import Align
from rich import box
from . import input_handler, config
from . import content_parser
from . import _rust
from . import cache as cache_mod

# ================================
# CENTRALIZED UI CONFIGURATION
# ================================
# Function to get current keyboard shortcuts
def get_keyboard_shortcuts():
    return input_handler.KEYBOARD_SHORTCUTS

class UIIcons:
    """Central place to configure all UI icons and separators."""
    
    # Status icons
    PLAYING = "▶"
    PAUSED = "⏸"
    
    # Mode icons
    AUTO_SCROLL = "▼"
    MANUAL_MODE = "⏹"
    
    # Navigation icons
    HIGHLIGHT_UP = "⇈"
    HIGHLIGHT_DOWN = "⇊"
    ROW_NAVIGATION = "↑↓"
    PAGE_NAVIGATION = "↑↓"
    QUIT = "⏻"
    
    # Separators
    SEPARATOR = "⸱"
    
    # Progress bar
    PROGRESS_FILLED = "▓"
    PROGRESS_EMPTY = "░"
    
    # Line separators for different widths
    LINE_SEPARATOR_LONG = "───"
    LINE_SEPARATOR_MEDIUM = "──"
    LINE_SEPARATOR_SHORT = "─"

class UIColors:
    """Central place to configure all UI colors and styles."""
    
    # Status colors
    PLAYING_STATUS = "green"
    PAUSED_STATUS = "yellow"
    
    # Mode colors
    AUTO_SCROLL_ENABLED = "magenta"
    AUTO_SCROLL_DISABLED = "blue"
    
    # Control and navigation colors
    CONTROL_KEYS = "white"          # h, j, k, l, etc.
    CONTROL_ICONS = "green"   # The actual navigation icons
    ARROW_ICONS = "blue"  # Color for u/n and i/m icons
    QUIT_ICON = "red"        # Color for the q icon
    SEPARATORS = "bright_blue"      # Lines and separators
    
    # Panel and UI structure
    PANEL_BORDER = "bright_blue"
    PANEL_TITLE = "bold blue"
    
    # Text content colors
    TEXT_NORMAL = "white"      # Normal reading text
    TEXT_HIGHLIGHT = "bold magenta" # Current sentence highlight
    WORD_HIGHLIGHT = "bold yellow"  # Current word highlight
    WORD_HIGHLIGHT_STANDOUT = "black on bright_yellow"  # Standout mode word highlight
    SPEED_READING_TEXT = "bold white"  # Centered speed reading word
    SELECTION_HIGHLIGHT = "reverse" # Text selection highlight
    
    # Progress bar colors
    PROGRESS_BAR = "bold blue"  # Used for the progress text in title
    
    # You can easily add theme presets here:
    @classmethod
    def apply_black_theme(cls):
        """Apply a dark theme color scheme with grayscale only."""
        # Dark theme base colors - using only grayscale
        cls.PLAYING_STATUS = "white"
        cls.PAUSED_STATUS = "white"
        cls.AUTO_SCROLL_ENABLED = "white"
        cls.AUTO_SCROLL_DISABLED = "white"
        cls.CONTROL_KEYS = "white"
        cls.CONTROL_ICONS = "white"
        cls.ARROW_ICONS = "white"
        cls.QUIT_ICON = "white"
        cls.SEPARATORS = "white"
        cls.PANEL_BORDER = "white"
        cls.PANEL_TITLE = "white"
        cls.PROGRESS_BAR = "white"
        cls.TEXT_NORMAL = "white"
        cls.TEXT_HIGHLIGHT = "grey70"
        cls.WORD_HIGHLIGHT = "white"
        cls.WORD_HIGHLIGHT_STANDOUT = "black on white"
        cls.SPEED_READING_TEXT = "bold white"
        cls.SELECTION_HIGHLIGHT = "on grey50"
    
    @classmethod
    def apply_white_theme(cls):
        """Apply a light theme color scheme with grayscale only."""
        # Light theme base colors - using only grayscale
        cls.PLAYING_STATUS = "black"
        cls.PAUSED_STATUS = "black"
        cls.AUTO_SCROLL_ENABLED = "black"
        cls.AUTO_SCROLL_DISABLED = "black"
        cls.CONTROL_KEYS = "black"
        cls.CONTROL_ICONS = "black"
        cls.ARROW_ICONS = "black"
        cls.QUIT_ICON = "black"
        cls.SEPARATORS = "black"
        cls.PANEL_BORDER = "black"
        cls.PANEL_TITLE = "black"
        cls.PROGRESS_BAR = "black"
        cls.TEXT_NORMAL = "black"
        cls.TEXT_HIGHLIGHT = "grey30"
        cls.WORD_HIGHLIGHT = "black"
        cls.WORD_HIGHLIGHT_STANDOUT = "white on black"
        cls.SPEED_READING_TEXT = "bold black"
        cls.SELECTION_HIGHLIGHT = "on grey50"
    
# Create global instances for easy access
ICONS = UIIcons()
COLORS = UIColors()

# Uncomment one of these to apply a different theme:
# COLORS.apply_black_theme()
# COLORS.apply_white_theme()

def get_terminal_size():
    """Get terminal size."""
    try:
        columns, rows = os.get_terminal_size()
        return columns, rows
    except OSError:
        return 80, 24

def update_document_layout(reader):
    """Update the document layout based on terminal size.

    布局构建（rich 的逐段 wrap 测量）在大文档上开销显著，而 UI 模式切换
    （v 键）只在 available_width 变化时才需要重建，因此按宽度缓存布局，
    命中时直接复用，消除模式切换时的卡顿。
    """
    width, _ = get_terminal_size()

    # Adjust available width based on UI mode
    if config.UI_MODE == 0 or config.UI_MODE == 3:
        # Mode 0 or 3: Full screen width for text
        available_width = width
    else:
        # Mode 1 and 2: Account for borders and padding
        available_width = max(20, width - 10)

    cache = getattr(reader, '_layout_cache', None)
    cached = cache.get(available_width) if cache else None
    disk_layout = None
    if not cached:
        # 布局磁盘缓存:同一书在同一宽度下跳过整书 wrap 重建(v 键/resize
        # 切换宽度会重建一次并写回;书内容变化按 mtime+size 失效)
        try:
            disk_layout = cache_mod.LayoutCache(
                reader.file_path,
                cache_mod.LayoutCache.key_for(os.stat(reader.file_path), available_width),
            ).load()
        except Exception:
            disk_layout = None
    if cached:
        reader.document_lines = cached['lines']
        reader.line_to_position = cached['line_to_position']
        reader.position_to_line = cached['position_to_line']
        reader.paragraph_line_ranges = cached['paragraph_ranges']
        reader._sorted_line_positions = cached['sorted_index']
    elif disk_layout is not None:
        # 与 rebuild 一致:document_lines 存普通 str,渲染时才转 Text
        reader.document_lines = disk_layout['lines']
        reader.line_to_position = disk_layout['line_to_position']
        reader.position_to_line = disk_layout['position_to_line']
        reader.paragraph_line_ranges = disk_layout['paragraph_ranges']
        reader._sorted_line_positions = disk_layout['sorted_index']
    else:
        # 布局重建:整章批量 wrap(一次 FFI 调用,替代逐段 dict 查找);
        # document_lines 先存普通行文本(缓存与二分子结构都更轻),
        # 需要渲染时才在 get_visible_content 转成 Text。
        blank_text = Text("", style=COLORS.TEXT_NORMAL)
        reader.document_lines = []
        reader.line_to_position = {}
        reader.position_to_line = {}
        reader.paragraph_line_ranges = {}

        wrap_paragraph = (
            _rust.lue_rs.wrap_paragraph
            if _rust.lue_rs is not None
            else None
        )

        for chap_idx, chapter in enumerate(reader.chapters):
            if chap_idx > 0:
                reader.document_lines.append("")

            for para_idx, paragraph in enumerate(chapter):
                paragraph_start_line = len(reader.document_lines)

                if wrap_paragraph is not None:
                    try:
                        wrapped_plains = wrap_paragraph(paragraph, available_width)
                    except _rust._PANIC_TYPES:
                        wrapped_plains = None
                else:
                    wrapped_plains = None
                if wrapped_plains is None:
                    plain_text = Text(paragraph, justify="left", no_wrap=False, style=COLORS.TEXT_NORMAL)
                    wrapped_plains = [line.plain for line in plain_text.wrap(reader.console, available_width)]

                paragraph_end_line = paragraph_start_line + len(wrapped_plains) - 1
                reader.paragraph_line_ranges[(chap_idx, para_idx)] = (paragraph_start_line, paragraph_end_line)

                sentences = content_parser.split_into_sentences(paragraph)
                # 句子起点与换行位置都单调递增,用双指针线性定位句首所在行,
                # 避免原实现的“每句从头扫描所有行”的 O(句数×行数) 开销
                line_idx = 0
                line_offset = 0
                current_char_pos = 0
                for sent_idx, sentence in enumerate(sentences):
                    sentence_start = current_char_pos
                    while (
                        line_idx < len(wrapped_plains)
                        and sentence_start >= line_offset + len(wrapped_plains[line_idx])
                    ):
                        line_offset += len(wrapped_plains[line_idx])
                        line_idx += 1
                    if line_idx < len(wrapped_plains):
                        reader.position_to_line[(chap_idx, para_idx, sent_idx)] = paragraph_start_line + line_idx
                    current_char_pos = sentence_start + len(sentence) + 1

                for line_idx in range(len(wrapped_plains)):
                    reader.line_to_position[paragraph_start_line + line_idx] = (chap_idx, para_idx, 0)

                reader.document_lines.extend(wrapped_plains)

                if para_idx < len(chapter) - 1:
                    reader.document_lines.append("")

        if cache is None:
            cache = reader._layout_cache = {}
        # 按行排序的 (positions, lines) 平行数组:_get_topmost_visible_sentence
        # 用二分替代 O(全书句子) 的 dict 扫描;稳定排序保持插入序,同一行的
        # 多个句子位置中取插入最早的那个,与原扫描语义一致
        sorted_items = sorted(reader.position_to_line.items(), key=lambda kv: kv[1])
        sorted_index = (
            [pos for pos, _ in sorted_items],
            [ln for _, ln in sorted_items],
        )
        reader._sorted_line_positions = sorted_index
        cache[available_width] = {
            'lines': reader.document_lines,
            'line_to_position': reader.line_to_position,
            'position_to_line': reader.position_to_line,
            'paragraph_ranges': reader.paragraph_line_ranges,
            'sorted_index': sorted_index,
        }
        # 持久化布局缓存(后台线程写盘,不阻塞渲染也不依赖事件循环
        # 状态;asyncio.get_event_loop 在无事件循环的同步加载期会抛错)
        try:
            import threading

            layout_payload = cache[available_width]

            def _persist_layout():
                try:
                    cache_mod.LayoutCache(
                        reader.file_path,
                        cache_mod.LayoutCache.key_for(
                            os.stat(reader.file_path), available_width
                        ),
                    ).store(layout_payload)
                except Exception:
                    pass

            threading.Thread(target=_persist_layout, daemon=True).start()
        except Exception:
            pass
        # 只保留最近使用的两个宽度，避免终端频繁 resize 时内存无限增长
        if len(cache) > 2:
            for old_width in sorted(cache.keys())[:-2]:
                del cache[old_width]

    if hasattr(reader, '_initial_load_complete') and reader._initial_load_complete:
        scroll_was_set = False
        if not reader.auto_scroll_enabled and reader.resize_anchor:
            anchor_pos = reader.resize_anchor
            if anchor_pos and anchor_pos in reader.position_to_line:
                target_line = reader.position_to_line[anchor_pos]
                _, height = get_terminal_size()
                available_height = max(1, height - 4)
                max_scroll = max(0, len(reader.document_lines) - available_height)
                reader.scroll_offset = reader.target_scroll_offset = min(target_line, max_scroll)
                scroll_was_set = True
            reader.resize_anchor = None

        if not scroll_was_set:
            current_position_key = (reader.ui_chapter_idx, reader.ui_paragraph_idx, reader.ui_sentence_idx)
            reader._scroll_to_position(
                current_position_key[0],
                current_position_key[1],
                current_position_key[2],
                smooth=False
            )


def get_current_word(reader):
    """Return the current spoken word for speed reading mode."""
    words = getattr(reader, 'current_sentence_words', None)
    word_idx = getattr(reader, 'ui_word_idx', 0)
    if words and 0 <= word_idx < len(words):
        return words[word_idx]

    try:
        paragraph = reader.chapters[reader.ui_chapter_idx][reader.ui_paragraph_idx]
        sentence = content_parser.split_into_sentences(paragraph)[reader.ui_sentence_idx]
    except (AttributeError, IndexError):
        return ""

    fallback_words = [token for token in sentence.split() if re.search(r'[a-zA-Z0-9]', token)]
    return fallback_words[0] if fallback_words else ""


def render_speed_reading_output(reader, width, height, console):
    """Render speed reading word centered in the current terminal size."""
    word = get_current_word(reader)
    center_y = max(0, ((height + 1) // 2) - 1)
    escaped_word = word.replace("\033", "")
    word_width = Text(escaped_word).cell_len
    if word_width > width:
        escaped_word = escaped_word[:width]
        word_width = Text(escaped_word).cell_len
    center_x = max(0, (width - word_width) // 2)

    padded_content = Text()
    for line_idx in range(height):
        if line_idx == center_y:
            padded_content.append(" " * center_x)
            padded_content.append(escaped_word, style=COLORS.SPEED_READING_TEXT)
            padded_content.append(" " * max(0, width - center_x - word_width))
        else:
            padded_content.append(" " * width)

        if line_idx < height - 1:
            padded_content.append("\n")

    with console.capture() as capture:
        console.print(padded_content, end='', overflow='crop')
    return capture.get()


def _apply_current_text_color(line):
    """Apply the current theme's text color to a line.

    document_lines 现在存普通 str(布局重建/磁盘缓存均不建 Text),渲染
    前在 get_visible_content 统一转 Text;这里同时兼容两者。
    """
    if isinstance(line, str):
        if not line:
            return Text("", style=COLORS.TEXT_NORMAL)
        return Text(line, justify="left", no_wrap=False, style=COLORS.TEXT_NORMAL)
    if not line.plain:
        return Text("", style=COLORS.TEXT_NORMAL)
    return Text(line.plain, justify="left", no_wrap=False, style=COLORS.TEXT_NORMAL)


def get_visible_content(reader):
    """Get the visible content to display."""
    width, height = get_terminal_size()
    
    # Adjust available space based on UI mode
    if config.UI_MODE == 0 or config.UI_MODE == 3:
        # Mode 0 or 3: Full screen for text, no borders or UI elements
        available_height = height
        available_width = width
    elif config.UI_MODE == 1:
        # Mode 1: Account for top title bar and borders, but no bottom controls
        available_height = max(1, height - 4)  # Top border, title, bottom border
        available_width = max(20, width - 10)  # Side borders and padding
    else:
        # Mode 2: Full UI with top and bottom elements
        available_height = max(1, height - 4)  # Top border, title, subtitle, bottom border
        available_width = max(20, width - 10)  # Side borders and padding

    start_line = int(reader.scroll_offset)
    end_line = min(len(reader.document_lines), start_line + available_height)

    visible_lines = []
    current_paragraph_key = (reader.ui_chapter_idx, reader.ui_paragraph_idx)

    highlighted_paragraph_lines = None
    if current_paragraph_key in reader.paragraph_line_ranges:
        para_start, para_end = reader.paragraph_line_ranges[current_paragraph_key]
        paragraph = reader.chapters[reader.ui_chapter_idx][reader.ui_paragraph_idx]
        sentences = content_parser.split_into_sentences(paragraph)
        highlighted_text = Text(justify="left", no_wrap=False)

        for sent_idx, sentence in enumerate(sentences):
            is_current_sentence = sent_idx == reader.ui_sentence_idx
            
            # Determine the base style for this sentence
            if is_current_sentence and config.SENTENCE_HIGHLIGHTING_ENABLED:
                base_style = COLORS.TEXT_HIGHLIGHT
            else:
                base_style = COLORS.TEXT_NORMAL
            
            # Apply word-level highlighting if enabled and this is the current sentence
            if (is_current_sentence and config.WORD_HIGHLIGHT_MODE > 0 and 
                hasattr(reader, 'ui_word_idx')):
                
                # Preserve leading whitespace from the sentence, which contains paragraph indentation
                leading_whitespace = ""
                if sentence:
                    match = re.match(r"^(\s+)", sentence)
                    if match:
                        leading_whitespace = match.group(1)
                
                if leading_whitespace:
                    highlighted_text.append(leading_whitespace, style=base_style)
                
                # Split sentence into tokens (preserving all original text)
                tokens = sentence.lstrip().split()
                
                # Track index of highlightable words only
                highlightable_word_count = 0
                
                for token_idx, token in enumerate(tokens):
                    # Split token on em dash or hyphen, keeping the separator as a separate part
                    sub_parts = re.split(r'([—-])', token)
                    
                    for part_idx, part in enumerate(sub_parts):
                        # If part is em dash, hyphen, or non-highlightable (no alnum), append without counting
                        if part in ['—', '-'] or not re.search(r'[a-zA-Z0-9]', part):
                            highlighted_text.append(part, style=base_style)
                        else:
                            # Highlightable word part
                            if highlightable_word_count == reader.ui_word_idx:
                                word_style = COLORS.WORD_HIGHLIGHT_STANDOUT if config.WORD_HIGHLIGHT_MODE == 2 else COLORS.WORD_HIGHLIGHT
                                highlighted_text.append(part, style=word_style)
                            else:
                                highlighted_text.append(part, style=base_style)
                            highlightable_word_count += 1
                    
                    # Add space after the full token (not between sub-parts)
                    if token_idx < len(tokens) - 1:
                        highlighted_text.append(" ", style=base_style)
            else:
                # No word highlighting, just apply the base style to the entire sentence
                highlighted_text.append(sentence, style=base_style)
            
            if sent_idx < len(sentences) - 1:
                highlighted_text.append(" ", style=COLORS.TEXT_NORMAL)

        highlighted_paragraph_lines = highlighted_text.wrap(reader.console, available_width)

    for i in range(start_line, end_line):
        if i < len(reader.document_lines):
            line = reader.document_lines[i]

            # Apply current theme text color
            line = _apply_current_text_color(line)

            if (i in reader.line_to_position and
                reader.line_to_position[i][:2] == current_paragraph_key and
                highlighted_paragraph_lines is not None):

                para_start, para_end = reader.paragraph_line_ranges[current_paragraph_key]
                line_offset = i - para_start

                if 0 <= line_offset < len(highlighted_paragraph_lines):
                    line = highlighted_paragraph_lines[line_offset]

            line = _apply_selection_highlighting(reader, line, i)

            visible_lines.append(line)
        else:
            visible_lines.append(Text("", style=COLORS.TEXT_NORMAL))

    if len(visible_lines) > available_height:
        visible_lines = visible_lines[:available_height]

    return visible_lines

def _apply_selection_highlighting(reader, line, line_index):
    """Apply selection highlighting to a line if it's within the selection range."""
    if not reader.selection_active or not reader.selection_start or not reader.selection_end:
        return line
    if isinstance(line, str):
        line = Text(line, justify="left", no_wrap=False, style=COLORS.TEXT_NORMAL)
    
    start_line, start_char = reader.selection_start
    end_line, end_char = reader.selection_end
    
    # Ensure start comes before end
    if start_line > end_line or (start_line == end_line and start_char > end_char):
        start_line, start_char, end_line, end_char = end_line, end_char, start_line, start_char
    
    # Check if this line is within the selection range
    if not (start_line <= line_index <= end_line):
        return line
    
    line_text = line.plain
    if not line_text:
        return line
    
    # Create a new Text object with selection highlighting
    new_line = Text(justify="left", no_wrap=False)
    
    if start_line == end_line == line_index:
        # Single line selection
        selection_start = max(0, min(start_char, len(line_text)))
        selection_end = max(0, min(end_char, len(line_text)))
        
        # Add text before selection
        if selection_start > 0:
            new_line.append(line_text[:selection_start], style=COLORS.TEXT_NORMAL)
        
        # Add selected text with highlighting
        if selection_end > selection_start:
            new_line.append(line_text[selection_start:selection_end], style=COLORS.SELECTION_HIGHLIGHT)
        
        # Add text after selection
        if selection_end < len(line_text):
            new_line.append(line_text[selection_end:], style=COLORS.TEXT_NORMAL)
            
    elif line_index == start_line:
        # First line of multi-line selection
        selection_start = max(0, min(start_char, len(line_text)))
        
        # Add text before selection
        if selection_start > 0:
            new_line.append(line_text[:selection_start], style=COLORS.TEXT_NORMAL)
        
        # Add selected text from start_char to end of line
        if selection_start < len(line_text):
            new_line.append(line_text[selection_start:], style=COLORS.SELECTION_HIGHLIGHT)
            
    elif line_index == end_line:
        # Last line of multi-line selection
        selection_end = max(0, min(end_char, len(line_text)))
        
        # Add selected text from beginning to end_char
        if selection_end > 0:
            new_line.append(line_text[:selection_end], style=COLORS.SELECTION_HIGHLIGHT)
        
        # Add text after selection
        if selection_end < len(line_text):
            new_line.append(line_text[selection_end:], style=COLORS.TEXT_NORMAL)
            
    else:
        # Middle line of multi-line selection - entire line is selected
        new_line.append(line_text, style=COLORS.SELECTION_HIGHLIGHT)
    
    return new_line


def _strip_rich_markup(text):
    """Strip Rich markup tags from a string, returning plain visual text."""
    return re.sub(r'\[/?[^\]]*\]', '', text)


def _compute_subtitle_hitboxes(segments, width):
    total_plain = sum(len(t) for _, t in segments)
    inner_width = width - 2
    start_x = 2 + max(0, (inner_width - total_plain) // 2)

    hitboxes = []
    cursor = start_x
    for key, plain in segments:
        seg_len = len(plain)
        if key is not None and seg_len > 0:
            hitboxes.append((key, cursor, cursor + seg_len - 1))
        cursor += seg_len
    return hitboxes


def get_compact_subtitle(reader, width):
    """Generate a compact subtitle based on terminal width."""
    status_icon = ICONS.PLAYING if not reader.is_paused else ICONS.PAUSED
    status_text = "PLAYING" if not reader.is_paused else "PAUSED"
    
    # Add speed indicator if not normal speed
    speed_indicator = reader._get_speed_display() if hasattr(reader, '_get_speed_display') else ""
    
    # Get keyboard shortcuts
    keyboard_shortcuts = get_keyboard_shortcuts()
    nav_shortcuts = keyboard_shortcuts.get("navigation", {})
    tts_shortcuts = keyboard_shortcuts.get("tts_controls", {})
    display_shortcuts = keyboard_shortcuts.get("display_controls", {})
    app_shortcuts = keyboard_shortcuts.get("application", {})
    
    # Control text with centralized colors using loaded shortcuts
    # Apply formatting to make control characters readable
    prev_para_key = format_key_for_display(nav_shortcuts.get("prev_paragraph", "h"))
    next_para_key = format_key_for_display(nav_shortcuts.get("next_paragraph", "l"))
    prev_sent_key = format_key_for_display(nav_shortcuts.get("prev_sentence", "j"))
    next_sent_key = format_key_for_display(nav_shortcuts.get("next_sentence", "k"))
    scroll_up_key = format_key_for_display(nav_shortcuts.get("scroll_up", "u"))
    scroll_down_key = format_key_for_display(nav_shortcuts.get("scroll_down", "n"))
    page_up_key = format_key_for_display(nav_shortcuts.get("scroll_page_up", "i"))
    page_down_key = format_key_for_display(nav_shortcuts.get("scroll_page_down", "m"))
    quit_key = format_key_for_display(app_shortcuts.get("quit", "q"))
    auto_scroll_key = format_key_for_display(display_shortcuts.get("toggle_auto_scroll", "a"))
    top_visible_key = format_key_for_display(nav_shortcuts.get("move_to_top_visible", "t"))
    
    nav_text_1 = f"[{COLORS.CONTROL_KEYS}]{prev_para_key}{ICONS.SEPARATOR}{prev_sent_key}[/{COLORS.CONTROL_KEYS}]"
    nav_text_2 = f"[{COLORS.CONTROL_KEYS}]{next_sent_key}{ICONS.SEPARATOR}{next_para_key}[/{COLORS.CONTROL_KEYS}]"
    page_text = f"[{COLORS.CONTROL_KEYS}]{scroll_up_key}{ICONS.SEPARATOR}{scroll_down_key}[/{COLORS.CONTROL_KEYS}]"
    scroll_text = f"[{COLORS.CONTROL_KEYS}]{page_up_key}{ICONS.SEPARATOR}{page_down_key}[/{COLORS.CONTROL_KEYS}]"
    quit_text = f"[{COLORS.CONTROL_KEYS}]{quit_key}[/{COLORS.CONTROL_KEYS}]"
    auto_text = f"[{COLORS.CONTROL_KEYS}]{auto_scroll_key}{ICONS.SEPARATOR}{top_visible_key}[/{COLORS.CONTROL_KEYS}]"
    # Removed ui_mode_text from here as we don't want to show it visually
    
    if reader.auto_scroll_enabled:
        auto_scroll_icon = ICONS.AUTO_SCROLL
        auto_scroll_text = "AUTO"
    else:
        auto_scroll_icon = ICONS.MANUAL_MODE
        auto_scroll_text = "MANUAL"
    
    if width >= 80:
        base_sep = ICONS.LINE_SEPARATOR_LONG
        
        # Construct status part with proper spacing
        pause_key = format_key_for_display(tts_shortcuts.get("play_pause", "p"))
        if speed_indicator:
            status_part = f"[{COLORS.CONTROL_KEYS}]{pause_key}[/{COLORS.CONTROL_KEYS}] {status_icon} {speed_indicator} {status_text}"
        else:
            status_part = f"[{COLORS.CONTROL_KEYS}]{pause_key}[/{COLORS.CONTROL_KEYS}] {status_icon} {status_text}"
            
        status_extra = 1 if status_text == "PAUSED" else 0
        status_sep = base_sep + (ICONS.LINE_SEPARATOR_SHORT * status_extra)
        
        auto_part = f"{auto_scroll_icon} {auto_scroll_text}"
        auto_extra = 2 if auto_scroll_text == "AUTO" else 0
        auto_sep = base_sep + (ICONS.LINE_SEPARATOR_SHORT * auto_extra)
        
        # Get UI mode display
        ui_mode_names = ["MIN", "MED", "FULL", "SR"]
        ui_mode_display = ui_mode_names[config.UI_MODE]
        
        # Modified controls_text to remove ui_mode_text visual display but keep functionality
        controls_text = f"{nav_text_1} [{COLORS.CONTROL_ICONS}]{ICONS.HIGHLIGHT_UP}[/{COLORS.CONTROL_ICONS}] {nav_text_2} [{COLORS.CONTROL_ICONS}]{ICONS.HIGHLIGHT_DOWN}[/{COLORS.CONTROL_ICONS}] [{COLORS.SEPARATORS}]{base_sep}[/{COLORS.SEPARATORS}] {page_text} [{COLORS.ARROW_ICONS}]{ICONS.ROW_NAVIGATION}[/{COLORS.ARROW_ICONS}] {scroll_text} [{COLORS.ARROW_ICONS}]{ICONS.PAGE_NAVIGATION}[/{COLORS.ARROW_ICONS}] [{COLORS.SEPARATORS}]{base_sep}[/{COLORS.SEPARATORS}] {quit_text} [{COLORS.QUIT_ICON}]{ICONS.QUIT}[/{COLORS.QUIT_ICON}]"
        
        playing_color = COLORS.PLAYING_STATUS if not reader.is_paused else COLORS.PAUSED_STATUS
        auto_color = COLORS.AUTO_SCROLL_ENABLED if reader.auto_scroll_enabled else COLORS.AUTO_SCROLL_DISABLED
        
        rich_result = (
            f"[{playing_color}]{status_part}[/{playing_color}] "
            f"[{COLORS.SEPARATORS}]{status_sep}[/{COLORS.SEPARATORS}] "
            f"{auto_text} "
            f"[{auto_color}]{auto_part}[/{auto_color}] "
            f"[{COLORS.SEPARATORS}]{auto_sep}[/{COLORS.SEPARATORS}] "
            f"{controls_text}"
        )

        p_status = _strip_rich_markup(status_part)
        p_status_sep = f" {status_sep} "
        p_auto_part = auto_part + " "
        p_auto_sep = f"{auto_sep} "

        segments = [
            ('pause',                p_status),
            (None,                   p_status_sep),
            ('toggle_auto_scroll',   auto_scroll_key),
            (None,                   ICONS.SEPARATOR),
            ('move_to_top_visible',  top_visible_key + " "),
            ('toggle_auto_scroll',   p_auto_part),
            (None,                   p_auto_sep),
            ('prev_paragraph',       f"{prev_para_key}{ICONS.SEPARATOR}"),
            ('prev_sentence',        f"{prev_sent_key}"),
            (None,                   f" {ICONS.HIGHLIGHT_UP} "),
            ('next_sentence',        f"{next_sent_key}{ICONS.SEPARATOR}"),
            ('next_paragraph',       f"{next_para_key}"),
            (None,                   f" {ICONS.HIGHLIGHT_DOWN} {base_sep} "),
            ('scroll_up',            f"{scroll_up_key}{ICONS.SEPARATOR}"),
            ('scroll_down',          f"{scroll_down_key}"),
            (None,                   f" {ICONS.ROW_NAVIGATION} "),
            ('scroll_page_up',       f"{page_up_key}{ICONS.SEPARATOR}"),
            ('scroll_page_down',     f"{page_down_key}"),
            (None,                   f" {ICONS.PAGE_NAVIGATION} {base_sep} "),
            ('quit',                 f"{quit_key} {ICONS.QUIT}"),
        ]
        reader.subtitle_hitboxes = _compute_subtitle_hitboxes(segments, width)
        return rich_result
    elif width >= 70:
        separator = ICONS.LINE_SEPARATOR_LONG
        
        # Construct status part with proper spacing
        pause_key = format_key_for_display(tts_shortcuts.get("play_pause", "p"))
        if speed_indicator:
            icon_status = f"[{COLORS.CONTROL_KEYS}]{pause_key}[/{COLORS.CONTROL_KEYS}] {status_icon}{speed_indicator}"
        else:
            icon_status = f"[{COLORS.CONTROL_KEYS}]{pause_key}[/{COLORS.CONTROL_KEYS}] {status_icon}"
            
        icon_auto = f"{auto_scroll_icon}"
        
        # Get UI mode display
        ui_mode_names = ["MIN", "MED", "FULL", "SR"]
        ui_mode_display = ui_mode_names[config.UI_MODE]
        
        # Modified controls_text to remove ui_mode_text visual display but keep functionality
        controls_text = f"[{COLORS.SEPARATORS}]{separator}[/{COLORS.SEPARATORS}] {nav_text_1} [{COLORS.CONTROL_ICONS}]{ICONS.HIGHLIGHT_UP}[/{COLORS.CONTROL_ICONS}] {nav_text_2} [{COLORS.CONTROL_ICONS}]{ICONS.HIGHLIGHT_DOWN}[/{COLORS.CONTROL_ICONS}] [{COLORS.SEPARATORS}]{separator}[/{COLORS.SEPARATORS}] {page_text} [{COLORS.ARROW_ICONS}]{ICONS.ROW_NAVIGATION}[/{COLORS.ARROW_ICONS}] {scroll_text} [{COLORS.ARROW_ICONS}]{ICONS.PAGE_NAVIGATION}[/{COLORS.ARROW_ICONS}] [{COLORS.SEPARATORS}]{separator}[/{COLORS.SEPARATORS}] {quit_text} [{COLORS.QUIT_ICON}]{ICONS.QUIT}[/{COLORS.QUIT_ICON}]"
        
        playing_color = COLORS.PLAYING_STATUS if not reader.is_paused else COLORS.PAUSED_STATUS
        auto_color = COLORS.AUTO_SCROLL_ENABLED if reader.auto_scroll_enabled else COLORS.AUTO_SCROLL_DISABLED
        
        rich_result = f"[{playing_color}]{icon_status}[/{playing_color}] [{COLORS.SEPARATORS}]{separator}[/{COLORS.SEPARATORS}] {auto_text} [{auto_color}]{icon_auto}[/{auto_color}] {controls_text}"
        segments = [
            ('pause',                _strip_rich_markup(icon_status)),
            (None,                   f" {separator} "),
            ('toggle_auto_scroll',   auto_scroll_key),
            (None,                   ICONS.SEPARATOR),
            ('move_to_top_visible',  top_visible_key + " "),
            ('toggle_auto_scroll',   icon_auto),
            (None,                   f" {separator} "),
            ('prev_paragraph',       f"{prev_para_key}{ICONS.SEPARATOR}"),
            ('prev_sentence',        f"{prev_sent_key}"),
            (None,                   f" {ICONS.HIGHLIGHT_UP} "),
            ('next_sentence',        f"{next_sent_key}{ICONS.SEPARATOR}"),
            ('next_paragraph',       f"{next_para_key}"),
            (None,                   f" {ICONS.HIGHLIGHT_DOWN} {separator} "),
            ('scroll_up',            f"{scroll_up_key}{ICONS.SEPARATOR}"),
            ('scroll_down',          f"{scroll_down_key}"),
            (None,                   f" {ICONS.ROW_NAVIGATION} "),
            ('scroll_page_up',       f"{page_up_key}{ICONS.SEPARATOR}"),
            ('scroll_page_down',     f"{page_down_key}"),
            (None,                   f" {ICONS.PAGE_NAVIGATION} {separator} "),
            ('quit',                 f"{quit_key} {ICONS.QUIT}"),
        ]
        reader.subtitle_hitboxes = _compute_subtitle_hitboxes(segments, width)
        return rich_result
    elif width >= 65:
        separator = ICONS.LINE_SEPARATOR_MEDIUM
        
        # Construct status part with proper spacing
        pause_key = format_key_for_display(tts_shortcuts.get("play_pause", "p"))
        if speed_indicator:
            icon_status = f"[{COLORS.CONTROL_KEYS}]{pause_key}[/{COLORS.CONTROL_KEYS}] {status_icon}{speed_indicator}"
        else:
            icon_status = f"[{COLORS.CONTROL_KEYS}]{pause_key}[/{COLORS.CONTROL_KEYS}] {status_icon}"
            
        icon_auto = f"{auto_scroll_icon}"
        
        # Get UI mode display
        ui_mode_names = ["MIN", "MED", "FULL", "SR"]
        ui_mode_display = ui_mode_names[config.UI_MODE]
        
        # Modified controls_text to remove ui_mode_text visual display but keep functionality
        controls_text = f"[{COLORS.SEPARATORS}]{separator}[/{COLORS.SEPARATORS}] {nav_text_1} [{COLORS.CONTROL_ICONS}]{ICONS.HIGHLIGHT_UP}[/{COLORS.CONTROL_ICONS}] {nav_text_2} [{COLORS.CONTROL_ICONS}]{ICONS.HIGHLIGHT_DOWN}[/{COLORS.CONTROL_ICONS}] [{COLORS.SEPARATORS}]{separator}[/{COLORS.SEPARATORS}] {page_text} [{COLORS.ARROW_ICONS}]{ICONS.ROW_NAVIGATION}[/{COLORS.ARROW_ICONS}] {scroll_text} [{COLORS.ARROW_ICONS}]{ICONS.PAGE_NAVIGATION}[/{COLORS.ARROW_ICONS}] [{COLORS.SEPARATORS}]{separator}[/{COLORS.SEPARATORS}] {quit_text} [{COLORS.QUIT_ICON}]{ICONS.QUIT}[/{COLORS.QUIT_ICON}]"
        
        playing_color = COLORS.PLAYING_STATUS if not reader.is_paused else COLORS.PAUSED_STATUS
        auto_color = COLORS.AUTO_SCROLL_ENABLED if reader.auto_scroll_enabled else COLORS.AUTO_SCROLL_DISABLED
        
        rich_result = f"[{playing_color}]{icon_status}[/{playing_color}] [{COLORS.SEPARATORS}]{separator}[/{COLORS.SEPARATORS}] {auto_text} [{auto_color}]{icon_auto}[/{auto_color}] {controls_text}"
        segments = [
            ('pause',                _strip_rich_markup(icon_status)),
            (None,                   f" {separator} "),
            ('toggle_auto_scroll',   auto_scroll_key),
            (None,                   ICONS.SEPARATOR),
            ('move_to_top_visible',  top_visible_key + " "),
            ('toggle_auto_scroll',   icon_auto),
            (None,                   f" {separator} "),
            ('prev_paragraph',       f"{prev_para_key}{ICONS.SEPARATOR}"),
            ('prev_sentence',        f"{prev_sent_key}"),
            (None,                   f" {ICONS.HIGHLIGHT_UP} "),
            ('next_sentence',        f"{next_sent_key}{ICONS.SEPARATOR}"),
            ('next_paragraph',       f"{next_para_key}"),
            (None,                   f" {ICONS.HIGHLIGHT_DOWN} {separator} "),
            ('scroll_up',            f"{scroll_up_key}{ICONS.SEPARATOR}"),
            ('scroll_down',          f"{scroll_down_key}"),
            (None,                   f" {ICONS.ROW_NAVIGATION} "),
            ('scroll_page_up',       f"{page_up_key}{ICONS.SEPARATOR}"),
            ('scroll_page_down',     f"{page_down_key}"),
            (None,                   f" {ICONS.PAGE_NAVIGATION} {separator} "),
            ('quit',                 f"{quit_key} {ICONS.QUIT}"),
        ]
        reader.subtitle_hitboxes = _compute_subtitle_hitboxes(segments, width)
        return rich_result
    else:
        separator = ICONS.LINE_SEPARATOR_SHORT
        
        # Construct status part with proper spacing
        pause_key = format_key_for_display(tts_shortcuts.get("play_pause", "p"))
        if speed_indicator:
            icon_status = f"[{COLORS.CONTROL_KEYS}]{pause_key}[/{COLORS.CONTROL_KEYS}] {status_icon}{speed_indicator}"
        else:
            icon_status = f"[{COLORS.CONTROL_KEYS}]{pause_key}[/{COLORS.CONTROL_KEYS}] {status_icon}"
            
        icon_auto = f"{auto_scroll_icon}"
        
        # Get UI mode display
        ui_mode_names = ["MIN", "MED", "FULL", "SR"]
        ui_mode_display = ui_mode_names[config.UI_MODE]
        
        # Modified controls_text to remove ui_mode_text visual display but keep functionality
        controls_text = f"[{COLORS.SEPARATORS}]{separator}[/{COLORS.SEPARATORS}] {nav_text_1} [{COLORS.CONTROL_ICONS}]{ICONS.HIGHLIGHT_UP}[/{COLORS.CONTROL_ICONS}] {nav_text_2} [{COLORS.CONTROL_ICONS}]{ICONS.HIGHLIGHT_DOWN}[/{COLORS.CONTROL_ICONS}] [{COLORS.SEPARATORS}]{separator}[/{COLORS.SEPARATORS}] {page_text} [{COLORS.ARROW_ICONS}]{ICONS.ROW_NAVIGATION}[/{COLORS.ARROW_ICONS}] {scroll_text} [{COLORS.ARROW_ICONS}]{ICONS.PAGE_NAVIGATION}[/{COLORS.ARROW_ICONS}] [{COLORS.SEPARATORS}]{separator}[/{COLORS.SEPARATORS}] {quit_text} [{COLORS.QUIT_ICON}]{ICONS.QUIT}[/{COLORS.QUIT_ICON}]"
        
        playing_color = COLORS.PLAYING_STATUS if not reader.is_paused else COLORS.PAUSED_STATUS
        auto_color = COLORS.AUTO_SCROLL_ENABLED if reader.auto_scroll_enabled else COLORS.AUTO_SCROLL_DISABLED
        
        rich_result = f"[{playing_color}]{icon_status}[/{playing_color}] [{COLORS.SEPARATORS}]{separator}[/{COLORS.SEPARATORS}] {auto_text} [{auto_color}]{icon_auto}[/{auto_color}] {controls_text}"
        segments = [
            ('pause',                _strip_rich_markup(icon_status)),
            (None,                   f" {separator} "),
            ('toggle_auto_scroll',   auto_scroll_key),
            (None,                   ICONS.SEPARATOR),
            ('move_to_top_visible',  top_visible_key + " "),
            ('toggle_auto_scroll',   icon_auto),
            (None,                   f" {separator} "),
            ('prev_paragraph',       f"{prev_para_key}{ICONS.SEPARATOR}"),
            ('prev_sentence',        f"{prev_sent_key}"),
            (None,                   f" {ICONS.HIGHLIGHT_UP} "),
            ('next_sentence',        f"{next_sent_key}{ICONS.SEPARATOR}"),
            ('next_paragraph',       f"{next_para_key}"),
            (None,                   f" {ICONS.HIGHLIGHT_DOWN} {separator} "),
            ('scroll_up',            f"{scroll_up_key}{ICONS.SEPARATOR}"),
            ('scroll_down',          f"{scroll_down_key}"),
            (None,                   f" {ICONS.ROW_NAVIGATION} "),
            ('scroll_page_up',       f"{page_up_key}{ICONS.SEPARATOR}"),
            ('scroll_page_down',     f"{page_down_key}"),
            (None,                   f" {ICONS.PAGE_NAVIGATION} {separator} "),
            ('quit',                 f"{quit_key} {ICONS.QUIT}"),
        ]
        reader.subtitle_hitboxes = _compute_subtitle_hitboxes(segments, width)
        return rich_result

def render_recent_books_overlay(reader, width, height):
    """Render the recent books overlay."""
    if not reader.recent_books_list:
        content = Text("No recent books found.", justify="center", style=COLORS.TEXT_NORMAL)
    else:
        table = Table(box=None, show_header=False, padding=0, expand=True)
        table.add_column("Selection", width=3)
        table.add_column("Title", ratio=1, no_wrap=True, overflow="ellipsis")
        table.add_column("Spacer", width=1)
        table.add_column("Progress", justify="right")
        
        for i, book in enumerate(reader.recent_books_list):
            is_selected = i == reader.recent_menu_selection_idx
            
            if is_selected:
                style = "reverse bold cyan"
                prefix = ">"
            else:
                style = COLORS.TEXT_NORMAL
                prefix = " "
                
            title = book['title']
            percentage = f"{int(book['percentage'])}%"
            
            table.add_row(
                prefix,
                Text(title, overflow="ellipsis", no_wrap=True),
                " ",
                percentage,
                style=style
            )
        content = table

    panel_width = min(60, width - 4)
    panel_height = min(len(reader.recent_books_list) + 4, height - 4)
    
    panel = Panel(
        content,
        title="[bold cyan]Recently Read[/bold cyan]",
        border_style="cyan",
        box=box.ROUNDED,
        width=panel_width,
        height=panel_height,
        padding=(1, 1)
    )
    
    return panel, panel_width, panel_height

def render_chapter_index_overlay(reader, width, height):
    """Render the chapter index overlay for navigation."""
    sel = reader.chapter_index_selection_idx
    scroll_offset = reader.chapter_index_scroll_offset
    total = len(reader.chapters)
    panel_height = min(total + 4, height - 4)
    max_visible = max(1, panel_height - 4)

    table = Table(box=None, show_header=False, padding=0, expand=True)
    table.add_column("Selection", width=2)
    table.add_column("Current", width=2)
    table.add_column("Title", ratio=1, no_wrap=True, overflow="ellipsis")

    for i in range(scroll_offset, min(scroll_offset + max_visible, total)):
        chapter = reader.chapters[i]
        is_selected = i == sel
        is_current = i == reader.chapter_idx

        if is_selected:
            style = "reverse bold cyan"
            prefix = ">"
        else:
            style = COLORS.TEXT_NORMAL
            prefix = " "
        current_marker = "*" if is_current else " "

        title = f"Chapter {i + 1}"
        for para in chapter:
            stripped = para.strip()
            if stripped:
                # 章节首段通常是标题行（如“第五章”/“尾声”），直接采用
                title = stripped
                if len(title) > 60:
                    title = title[:57] + "..."
                break

        table.add_row(
            prefix,
            current_marker,
            Text(title, overflow="ellipsis", no_wrap=True),
            style=style
        )

    panel = Panel(
        table,
        title="[bold cyan]Chapters[/bold cyan]",
        border_style="cyan",
        box=box.ROUNDED,
        width=min(60, width - 4),
        height=panel_height,
        padding=(1, 1)
    )

    return panel, min(60, width - 4), panel_height

async def display_ui(reader):
    """Display the UI."""
    if reader.render_lock.locked():
        return
    
    async with reader.render_lock:
        try:
            width, height = get_terminal_size()
            
            progress_percent = reader._calculate_ui_progress_percentage()
            rounded_scroll = round(reader.scroll_offset, 1)
            current_state = (
                reader.ui_chapter_idx, reader.ui_paragraph_idx, reader.ui_sentence_idx,
                getattr(reader, 'ui_word_idx', 0),  # Add word index to trigger UI updates
                rounded_scroll, reader.is_paused, int(progress_percent),
                width, height, reader.auto_scroll_enabled, reader.selection_active,
                reader.selection_start, reader.selection_end,
                # Add playback speed to trigger UI updates when speed changes
                reader.playback_speed, getattr(reader, 'speed_reading_enabled', False), config.UI_MODE,
                # Add recent menu state to trigger updates
                reader.show_recent_menu, reader.recent_menu_selection_idx,
                # Add chapter index state to trigger updates
                reader.show_chapter_index, reader.chapter_index_selection_idx
            )
            
            if reader.last_rendered_state == current_state and reader.last_terminal_size == (width, height):
                return
            
            reader.last_rendered_state = current_state
            reader.last_terminal_size = (width, height)
            
            # Start building the full output buffer
            # Move cursor to top-left and hide cursor
            # We avoid clearing the whole screen (\033[2J) to prevent flickering
            full_output = '\033[?25l\033[H'
            
            temp_console = Console(width=width, height=height, force_terminal=True)
            
            book_output = ""
            
            if getattr(reader, 'speed_reading_enabled', False):
                book_output = render_speed_reading_output(reader, width, height, temp_console)
            else:
                visible_lines = get_visible_content(reader)

            if getattr(reader, 'speed_reading_enabled', False):
                pass
            elif config.UI_MODE == 0 or config.UI_MODE == 3:
                # Mode 0 or 3: Minimal - text only, no borders
                # Manually pad lines to overwrite screen content (prevents ghosting without clearing screen)
                padded_content = Text()
                
                for i in range(height):
                    if i < len(visible_lines):
                        line = visible_lines[i].copy()
                        pad_len = width - line.cell_len
                        if pad_len > 0:
                            line.append(" " * pad_len)
                        padded_content.append(line)
                    else:
                        padded_content.append(" " * width)
                    
                    if i < height - 1:
                        padded_content.append("\n")
                
                with temp_console.capture() as capture:
                    temp_console.print(padded_content, end='', overflow='crop')
                
                book_output = capture.get()
                
            elif config.UI_MODE == 1:
                # Build book_content for Mode 1 and 2
                book_content = Text("")
                for i, line in enumerate(visible_lines):
                    book_content.append(line)
                    if i < len(visible_lines) - 1:
                        book_content.append("\n")

                # Mode 1: Medium - top bar with title and progress, borders, no bottom controls
                progress_bar_width = 10
                filled_blocks = int((progress_percent / 100) * progress_bar_width)
                empty_blocks = progress_bar_width - filled_blocks
                progress_bar = ICONS.PROGRESS_FILLED * filled_blocks + ICONS.PROGRESS_EMPTY * empty_blocks
                
                percentage_text = f"{int(progress_percent)}% {progress_bar}"
                
                available_width = width - len(percentage_text) - 6
                
                if len(reader.book_title) > available_width:
                    title_text = f"{reader.book_title[:available_width-3]}..."
                else:
                    title_text = reader.book_title
                
                used_space = len(title_text) + len(percentage_text) + 2
                remaining_space = width - used_space - 6
                connecting_line = ICONS.LINE_SEPARATOR_SHORT * max(0, remaining_space)
                
                progress_text = f"{title_text} {connecting_line} {percentage_text}"
                
                # Create a solid border line for the bottom
                # Using a simple approach that matches mode 2 behavior
                book_panel = Panel(
                    book_content,
                    title=f"[{COLORS.PANEL_TITLE}]{progress_text}[/{COLORS.PANEL_TITLE}]",
                    subtitle="",  # Empty subtitle to avoid border issues
                    border_style=COLORS.PANEL_BORDER,
                    padding=(1, 4),  # Same padding as mode 2 for consistency
                    title_align="center",
                    subtitle_align="center",
                    width=width,
                    height=height,
                    expand=False
                )
                
                with temp_console.capture() as capture:
                    temp_console.print(book_panel, end='', overflow='crop')
                
                book_output = capture.get()
                output_lines = book_output.split('\n')
                if len(output_lines) > height:
                    output_lines = output_lines[:height]
                    book_output = '\n'.join(output_lines)
                
            else:
                # Build book_content for Mode 1 and 2
                book_content = Text("")
                for i, line in enumerate(visible_lines):
                    book_content.append(line)
                    if i < len(visible_lines) - 1:
                        book_content.append("\n")

                # Mode 2: Full - default mode with all UI elements
                progress_bar_width = 10
                filled_blocks = int((progress_percent / 100) * progress_bar_width)
                empty_blocks = progress_bar_width - filled_blocks
                progress_bar = ICONS.PROGRESS_FILLED * filled_blocks + ICONS.PROGRESS_EMPTY * empty_blocks
                
                percentage_text = f"{int(progress_percent)}% {progress_bar}"
                
                available_width = width - len(percentage_text) - 6
                
                if len(reader.book_title) > available_width:
                    title_text = f"{reader.book_title[:available_width-3]}..."
                else:
                    title_text = reader.book_title
                
                used_space = len(title_text) + len(percentage_text) + 2
                remaining_space = width - used_space - 6
                connecting_line = ICONS.LINE_SEPARATOR_SHORT * max(0, remaining_space)
                
                progress_text = f"{title_text} {connecting_line} {percentage_text}"
                
                subtitle = get_compact_subtitle(reader, width)
                
                book_panel = Panel(
                    book_content,
                    title=f"[{COLORS.PANEL_TITLE}]{progress_text}[/{COLORS.PANEL_TITLE}]",
                    subtitle=subtitle,
                    border_style=COLORS.PANEL_BORDER,
                    padding=(1, 4),
                    title_align="center",
                    subtitle_align="center",
                    width=width,
                    height=height,
                    expand=False
                )
                
                with temp_console.capture() as capture:
                    temp_console.print(book_panel, end='', overflow='crop')
                
                book_output = capture.get()
                output_lines = book_output.split('\n')
                if len(output_lines) > height:
                    output_lines = output_lines[:height]
                    book_output = '\n'.join(output_lines)
            
            # Append book content to full output
            full_output += book_output
            
            # Overlay menu if needed
            if reader.show_recent_menu:
                menu_panel, panel_width, panel_height = render_recent_books_overlay(reader, width, height)
                with temp_console.capture() as capture:
                    temp_console.print(menu_panel, end='', overflow='crop')
                menu_output = capture.get()
                
                # Split menu output into lines
                menu_lines = menu_output.split('\n')
                
                start_y = (height - panel_height) // 2
                start_x = (width - panel_width) // 2
                
                # Construct overlay string using ANSI cursor movements
                overlay = ""
                for i, line in enumerate(menu_lines):
                    if i >= panel_height: break
                    # Move cursor to (start_y + i, start_x) - 1-based coordinates
                    overlay += f"\033[{start_y + i + 1};{start_x + 1}H{line}"
                
                full_output += overlay

            # Overlay chapter index if needed
            if reader.show_chapter_index:
                chapter_panel, panel_width, panel_height = render_chapter_index_overlay(reader, width, height)
                with temp_console.capture() as capture:
                    temp_console.print(chapter_panel, end='', overflow='crop')
                chapter_output = capture.get()

                chapter_lines = chapter_output.split('\n')

                start_y = (height - panel_height) // 2
                start_x = (width - panel_width) // 2

                # Store position info on reader for mouse click handling
                reader.chapter_index_panel_y = start_y
                reader.chapter_index_panel_x = start_x
                reader.chapter_index_panel_width = panel_width
                reader.chapter_index_panel_height = panel_height

                overlay = ""
                for i, line in enumerate(chapter_lines):
                    if i >= panel_height: break
                    overlay += f"\033[{start_y + i + 1};{start_x + 1}H{line}"

                full_output += overlay

            sys.stdout.write(full_output)
            sys.stdout.flush()
            
        except (IndexError, ValueError):
            pass

def _get_highlightable_words(sentence: str) -> list[str]:
    """
    Get list of words that should be considered for highlighting.
    
    This function filters out tokens that contain only punctuation/non-alphanumeric
    characters, which should not be counted as words for highlighting timing.
    
    Args:
        sentence: The sentence to process
        
    Returns:
        List of words that should be highlighted
    """
    # Split on whitespace to get tokens
    tokens = sentence.lstrip().split()
    
    # Filter out tokens that contain only punctuation/non-alphanumeric characters
    words = [token for token in tokens if re.search(r'[a-zA-Z0-9]', token)]
    
    return words

def _should_token_be_highlighted(token: str) -> bool:
    """
    Determine if a token should be highlighted as a word.
    
    Args:
        token: The token to evaluate
        
    Returns:
        True if token should be highlighted, False otherwise
    """
    return bool(re.search(r'[a-zA-Z0-9]', token))


def _extract_core_word(token: str) -> str:
    """
    Extract the core word from a token by removing surrounding punctuation.
    
    This function is more robust than simple strip() as it handles nested
    punctuation and preserves internal punctuation like contractions.
    
    Args:
        token: The token to process
        
    Returns:
        The core word without surrounding punctuation
    """
    if not token:
        return token
    
    # Remove leading punctuation
    start = 0
    while start < len(token) and not token[start].isalnum():
        start += 1
    
    # Remove trailing punctuation
    end = len(token) - 1
    while end >= start and not token[end].isalnum():
        end -= 1
    
    if start <= end:
        return token[start:end + 1]
    else:
        return ""

def format_key_for_display(key):
    """Convert control characters to caret notation for UI display."""
    if isinstance(key, list):
        if key:
            return format_key_for_display(key[0])
        return ""
    
    if isinstance(key, str) and len(key) == 1:
        char_code = ord(key)
        if 0 <= char_code <= 31:
            return f"^{chr(char_code + 96)}"
    
    return key
