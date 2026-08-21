import os
import re
import zipfile
import xml.etree.ElementTree as ET
import fitz
import markdown
from docx import Document
from striprtf.striprtf import rtf_to_text
import subprocess
import string
from functools import lru_cache
from rich.console import Console
from typing import List, Tuple
from pathlib import Path
from html.parser import HTMLParser
from html import unescape
from urllib.parse import unquote


@lru_cache(maxsize=8192)
def split_into_sentences(paragraph: str) -> tuple[str, ...]:
    """
    Splits a paragraph into sentences, intelligently handling common abbreviations and initials.

    Results are cached (callers only read the result; the tuple is immutable).
    Sentence splitting runs on every navigation key press and UI frame for the
    current paragraph, and over all paragraphs when rebuilding layout, so the
    cache removes a significant repeated cost for large books.
    """
    # A list of common English abbreviations that can be followed by a period.
    abbreviations = [
        "Mr", "Mrs", "Ms", "Dr", "Prof", "Rev", "Hon", "Jr", "Sr",
        "Cpl", "Sgt", "Gen", "Col", "Capt", "Lt", "Pvt",
        "vs", "viz", "etc", "eg", "ie",
        "Co", "Inc", "Ltd", "Corp",
        "St", "Ave", "Blvd"
    ]
    
    # Create a regex pattern for these abbreviations.
    abbrev_pattern = r"\b(" + "|".join(abbreviations) + r")\."
    
    # Use a unique placeholder that is highly unlikely to be in the original text.
    placeholder = "<LUE_PERIOD>"
    
    # 1. Protect periods in abbreviations by replacing them with the placeholder.
    paragraph = re.sub(abbrev_pattern, r"\1" + placeholder, paragraph, flags=re.IGNORECASE)
    
    # 2. Protect periods in initials (e.g., "J. F. Kennedy").
    # This looks for a single capital letter, a period, a space, and another capital letter.
    initial_pattern = r"\b([A-Z])\.(?=\s[A-Z])"
    paragraph = re.sub(initial_pattern, r"\1" + placeholder, paragraph)
    
    # 3. Split the text into sentences using the remaining punctuation.
    # English and Indic languages require whitespace after punctuation.
    # Chinese and Japanese do not require whitespace.
    # Note: Python's `re` module requires fixed-width lookbehinds. To support
    # optional closing quotes (0, 1, or 2) in no-space text, we unroll the conditions.
    spaced_punc = r'[.!?।॥]'
    cn_jp_punc = r'[。！？]'
    quotes = r'[”」』’"\']'
    split_pattern = (
        rf'(?<={spaced_punc})\s+|'                              # Spaced
        rf'(?<={cn_jp_punc})(?!{cn_jp_punc}|{quotes})\s*|'       # No-space: 0 quotes
        rf'(?<={cn_jp_punc}{quotes})(?!{cn_jp_punc})\s*|'        # No-space: 1 quote
        rf'(?<={cn_jp_punc}{quotes}{{2}})(?!{cn_jp_punc})\s*'    # No-space: 2 quotes
    )
    sentences = re.split(split_pattern, paragraph)
    
    # 4. Restore the periods and clean up the results.
    restored_sentences = []
    for sentence in sentences:
        if sentence:
            restored = sentence.replace(placeholder, ".")
            if restored:
                restored_sentences.append(restored)
                
    # If splitting resulted in an empty list, return the original paragraph as a single sentence.
    return restored_sentences if restored_sentences else [paragraph]


def sanitize_text_for_tts(text):
    """
    Sanitize text for TTS engines by removing special characters while preserving
    letters (including accented characters), numbers, and basic punctuation.
    
    Replaces em dashes (—) with comma-space for natural TTS pause.
    Replaces hyphens between alphanumeric characters with spaces for compound words.

    Args:
        text: The text to sanitize

    Returns:
        Sanitized text with special characters removed but Unicode letters preserved
    """
    if not text or not isinstance(text, str):
        return ""

    # Remove loose punctuation marks (periods, exclamation, question marks)
    # that are standalone (surrounded by whitespace or start/end of string).
    text = re.sub(r'(?:^|\s)[.!?]+(?=\s|$)', ' ', text)

    # Replace em and en dash with comma-space for natural pause in TTS
    text = re.sub(r'[—–]', ', ', text)
    
    # Replace hyphens between alphanumeric characters with spaces
    text = re.sub(r'(?<=\w)-(?=\w)', ' ', text)
    
    # Remove special characters but keep Unicode letters, numbers, and basic punctuation.
    # Include Chinese/Japanese punctuation and Indian Dandas (।॥).
    sanitized = re.sub(r"[^\w\s.,:'();?!。，！？；：、।॥-]", '', text, flags=re.UNICODE)
    
    # Collapse multiple spaces into single space
    sanitized = re.sub(r'\s+', ' ', sanitized)

    # Strip leading and trailing whitespace
    sanitized = sanitized.strip()

    return sanitized


def clean_visual_text(text):
    """
    Clean text for visual display in the UI.
    Focuses on markdown removal and whitespace normalization while preserving
    punctuation and special characters for proper display.
    """
    if not text or not isinstance(text, str):
        return text
    
    # Check if this is a code block line - if so, preserve it mostly as-is
    if text.startswith('__CODE_BLOCK__'):
        # Remove the marker and preserve the content with NO cleaning
        code_content = text[14:]  # Remove '__CODE_BLOCK__' marker

        # Preserve all formatting, spaces, and special characters for code
        return code_content
    
    # 1. Handle spaced dots - collapse patterns like " . . . " to "..."
    # First, handle sequences of 3 or more dots with any amount of spacing
    text = re.sub(r'\s*\.\s*\.\s*\.\s*(\.\s*)*', '...', text)  # " . . . " or more -> "..."
    # Then handle exactly 2 spaced dots
    text = re.sub(r'\s*\.\s*\.\s*(?!\s*\.)', '..', text)  # " . . " -> ".." (not followed by another dot)
    # Handle any remaining multiple consecutive dots
    text = re.sub(r'\.{4,}', '...', text)  # 4+ consecutive dots -> 3 dots
    
    # 2. Remove long sequences of repeated non-alphanumeric characters
    # But preserve bullet points (•), ellipsis (...), and common punctuation
    text = re.sub(r'[-_=~`^]{3,}', '', text)           # Remove ---- or ____ etc.
    text = re.sub(r'[*]{4,}', '', text)                # Remove **** but keep *** and less
    text = re.sub(r'[#]{4,}', '', text)                # Remove #### but keep ### and less
    text = re.sub(r'[+]{3,}', '', text)                # Remove +++ 
    text = re.sub(r'[|]{3,}', '', text)                # Remove |||
    text = re.sub(r'[\\]{3,}', '', text)               # Remove \
    text = re.sub(r'[/]{3,}', '', text)                # Remove ///
    
    # 3. Replace Unicode characters for better TTS
    unicode_replacements = {

        # Mathematical symbols -> text
        '×': ' multiplied by ', '÷': ' divided by ', '±': ' plus or minus ',
        '≤': ' less than or equal to ', '≥': ' greater than or equal to ', '≠': ' not equal to ',
        '≈': ' approximately' , '∞': 'infinity ', '%': ' percent ', '+': ' plus ', '=': ' equals ',

        # Other symbols -> text
        '°': ' degrees ', '™': ' trademark ', '®': ' registered ',
        '©': ' copyright ', '§': ' section ',

        # Replace curly apostrophe with basic apostrophe
        "’": "'",

        # Remove zero-width and invisible characters
        '\u200b': '', '\u200c': '', '\u200d': '',  # Zero-width spaces
        '\ufeff': '',  # Byte order mark
        '\u00ad': '',  # Soft hyphen
    }
    
    for old_char, new_char in unicode_replacements.items():
        text = text.replace(old_char, new_char)
    
    # 4. Handle ellipsis properly (before whitespace cleanup to avoid interference)
    text = re.sub(r'\.{4,}', '...', text)  # Multiple dots -> ellipsis
    text = re.sub(r'…+', '...', text)      # Unicode ellipsis -> ASCII ellipsis
    
    # 5. Clean up excessive whitespace
    text = re.sub(r'\s+', ' ', text)  # Multiple spaces -> single space
    text = re.sub(r'\n\s*\n\s*\n+', '\n\n', text)  # Multiple newlines -> double newline max
    
    # 6. Ensure proper spacing around ellipsis (after whitespace cleanup)
    text = re.sub(r'\.\.\.(?=\S)', '... ', text)  # Add space after ellipsis if missing
    
    # 7. Remove markdown formatting
    text = re.sub(r'\*\*([^*]+)\*\*', r'\1', text)  # **bold** -> bold
    text = re.sub(r'\*([^*]+)\*', r'\1', text)      # *italic* -> italic
    text = re.sub(r'__([^_]+)__', r'\1', text)      # __bold__ -> bold
    text = re.sub(r'_([^_]+)_', r'\1', text)        # _italic_ -> italic
    text = re.sub(r'`([^`]+)`', r'\1', text)        # `code` -> code
    text = re.sub(r'~~([^~]+)~~', r'\1', text)      # ~~strikethrough~~ -> strikethrough
    
    # Remove markdown links but keep the text
    text = re.sub(r'\[([^\]]+)\]\([^)]+\)', r'\1', text)  # [text](url) -> text
    text = re.sub(r'\[([^\]]+)\]\[[^\ ]*\]', r'\1', text)  # [text][ref] -> text
    
    # Remove reference link definitions
    text = re.sub(r'^\s*\[[^\ ]+\]:\s*\S+.*$', '', text, flags=re.MULTILINE)  # [ref]: url -> (remove)
    
    # Remove markdown headers (# symbols)
    text = re.sub(r'^#{1,6}\s+', '', text, flags=re.MULTILINE)  # # Header -> Header
    
    # 8. Fix common formatting issues (but preserve ellipsis)
    text = re.sub(r'\s+([,!?;:])', r'\1', text)  # Remove space before punctuation (except dots)
    text = re.sub(r'([,!?;:])\s*([,!?;:])', r'\1 \2', text)  # Ensure space between punctuation
    
    return text.strip()


class HTMLtoLines(HTMLParser):
    """HTML parser"""
    para = {"p", "div"}
    inde = {"q", "dt", "dd", "blockquote"}
    pref = {"pre"}
    bull = {"li"}
    hide = {"script", "style", "head"}

    def __init__(self):
        HTMLParser.__init__(self)
        self.text = [""]
        self.imgs = []
        self.ishead = False
        self.isinde = False
        self.isbull = False
        self.ispref = False
        self.ishidden = False
        self.idhead = set()
        self.idinde = set()
        self.idbull = set()
        self.idpref = set()
        self.hiding_tags = []  # Stack to track which tags are causing hiding
        self._current_span_attrs = {}

    def handle_starttag(self, tag, attrs):
        if re.match("h[1-6]", tag) is not None:
            self.ishead = True
        elif tag in self.inde:
            self.isinde = True
        elif tag in self.pref:
            self.ispref = True
        elif tag in self.bull:
            self.isbull = True
        elif tag in self.hide:
            self.ishidden = True
            self.hiding_tags.append(tag)
        elif tag == "sup":
            # Hide sup content for TTS
            self.ishidden = True
            self.hiding_tags.append("sup")
        elif tag == "sub":
            # Hide sub content for TTS
            self.ishidden = True
            self.hiding_tags.append("sub")
        elif tag == "span":
            # Store span attributes to check for common footnote patterns
            self._current_span_attrs = dict(attrs)
            # Check for common footnote-related class patterns
            span_class = self._current_span_attrs.get('class', '')
            if (len(span_class) <= 3 and  # Short class names are often footnote markers
                (span_class.isdigit() or  # Numeric classes
                 span_class in {'su', 'sit', 'bs', 'is', 'fn', 'note', 'ref'} or  # Common footnote classes
                 'footnote' in span_class.lower() or
                 'note' in span_class.lower() or
                 'ref' in span_class.lower())):
                self.ishidden = True
                self.hiding_tags.append("span")
        elif tag in {"img", "image"}:
            # Skip images completely - don't add anything to text
            pass

    def handle_startendtag(self, tag, attrs):
        if tag == "br":
            self.text += [""]
        elif tag in {"img", "image"}:
            # Skip images completely
            pass

    def handle_endtag(self, tag):
        if re.match("h[1-6]", tag) is not None:
            self.text.append("")
            self.text.append("")
            self.ishead = False
        elif tag in self.para:
            self.text.append("")
        elif tag in self.hide:
            if self.hiding_tags and self.hiding_tags[-1] == tag:
                self.hiding_tags.pop()
                self.ishidden = len(self.hiding_tags) > 0
        elif tag in self.inde:
            if self.text[-1] != "":
                self.text.append("")
            self.isinde = False
        elif tag in self.pref:
            if self.text[-1] != "":
                self.text.append("")
            self.ispref = False
        elif tag in self.bull:
            if self.text[-1] != "":
                self.text.append("")
            self.isbull = False
        elif tag == "sup":
            # End of sup tag - stop hiding content if this was the hiding tag
            if self.hiding_tags and self.hiding_tags[-1] == "sup":
                self.hiding_tags.pop()
                self.ishidden = len(self.hiding_tags) > 0
        elif tag == "sub":
            # End of sub tag - stop hiding content if this was the hiding tag
            if self.hiding_tags and self.hiding_tags[-1] == "sub":
                self.hiding_tags.pop()
                self.ishidden = len(self.hiding_tags) > 0
        elif tag == "span":
            # End of span tag - stop hiding content if this was a hiding span
            if self.hiding_tags and self.hiding_tags[-1] == "span":
                self.hiding_tags.pop()
                self.ishidden = len(self.hiding_tags) > 0
        elif tag in {"img", "image"}:
            # Skip images
            pass

    def handle_data(self, raw):
        if raw and not self.ishidden:
            if self.text[-1] == "":
                tmp = raw.lstrip()
            else:
                tmp = raw
            if self.ispref:
                line = unescape(tmp)
            else:
                line = unescape(re.sub(r"\s+", " ", tmp))
            self.text[-1] += line
            if self.ishead:
                self.idhead.add(len(self.text)-1)
            elif self.isbull:
                self.idbull.add(len(self.text)-1)
            elif self.inde:
                self.idinde.add(len(self.text)-1)
            elif self.ispref:
                self.idpref.add(len(self.text)-1)

    def get_lines(self):
        """Get clean text lines with proper formatting for different content types"""
        clean_lines = []
        for i, line in enumerate(self.text):
            line = line.strip()
            if line and len(line) > 3:  # Skip very short lines
                # Clean up footnote markers and image references
                line = self._clean_line(line)
                # Apply global visual text cleaning
                line = clean_visual_text(line)
                if line and len(line) > 3:  # Check again after cleaning
                    # Apply formatting based on content type
                    if i in self.idhead:
                        # Headers - add some visual separation
                        clean_lines.append("")
                        clean_lines.append(line)
                        clean_lines.append("")
                    elif i in self.idbull:
                        # List items - add bullet point
                        clean_lines.append(f"• {line}")
                    elif i in self.idinde:
                        # Indented content (blockquotes) - add indentation
                        clean_lines.append(f"    {line}")
                    elif i in self.idpref:
                        # Preformatted text (code blocks) - preserve as-is with indentation
                        clean_lines.append(f"    {line}")
                    else:
                        # Regular paragraphs
                        clean_lines.append(line)
        
        # Remove excessive empty lines (more than 2 consecutive)
        result = []
        empty_count = 0
        for line in clean_lines:
            if line == "":
                empty_count += 1
                if empty_count <= 2:  # Allow up to 2 consecutive empty lines
                    result.append(line)
            else:
                empty_count = 0
                result.append(line)
        
        return result
    
    def _is_footnote_reference(self, content):
        """
        Detect if content looks like a footnote reference based on patterns.
        This is more general than hardcoding specific class names.
        """
        if not content or len(content.strip()) == 0:
            return False
            
        content = content.strip()
        
        # Very short content that's just numbers, letters, or footnote symbols
        if len(content) <= 3:
            # Pure numbers (verse numbers, sentence numbers, footnote numbers)
            if re.match(r'^\d+$', content):
                return True
            # Footnote symbols
            if re.match(r'^[*†‡§¶]+$', content):
                return True
            # Single letters (footnote markers)
            if re.match(r'^[a-zA-Z]$', content):
                return True
        
        # Slightly longer but still footnote-like patterns
        if len(content) <= 5:
            # Numbers with punctuation
            if re.match(r'^\d+[.,;:]?$', content):
                return True
            # Roman numerals
            if re.match(r'^[ivxlcdm]+$', content.lower()):
                return True
        
        return False

    def _clean_line(self, line):
        """Clean footnote markers and image references from a line"""
        # Remove footnote markers like ^{12}, _{sub}
        line = re.sub(r'\^{[^}]*}', '', line)
        line = re.sub(r'_{[^}]*}', '', line)
        
        # Remove image references like [IMG:0]
        line = re.sub(r'\[IMG:\d+\]', '', line)
        
        # Remove bracketed footnote references
        line = re.sub(r'\[\d+\]', '', line)
        line = re.sub(r'\[[a-zA-Z]+\d*\]', '', line)
        
        # Remove footnote symbols
        line = re.sub(r'[*†‡§¶]+', '', line)
        
        # Remove superscript numbers (since we're handling sup tags at HTML level, 
        # any remaining ones are likely Unicode superscripts)
        line = re.sub(r'[¹²³⁴⁵⁶⁷⁸⁹⁰]+', '', line)
        
        # Clean up extra whitespace
        line = re.sub(r'\s+', ' ', line).strip()
        
        return line


def extract_content(file_path, console):
    """Extract content from the file based on its extension."""
    file_extension = os.path.splitext(file_path)[1].lower()
    if file_extension == '.epub':
        return _extract_content_epub(file_path, console)
    elif file_extension == '.pdf':
        return _extract_content_pdf(file_path, console)
    elif file_extension == '.txt':
        return _extract_content_txt(file_path, console)
    elif file_extension == '.docx':
        return _extract_content_docx(file_path, console)
    elif file_extension == '.html':
        return _extract_content_html(file_path, console)
    elif file_extension == '.rtf':
        return _extract_content_rtf(file_path, console)
    elif file_extension == '.md':
        return _extract_content_md(file_path, console)
    else:
        console.print(f"[bold red]Error: Unsupported file type '{file_extension}'. "
                     f"Supported formats: .epub, .pdf, .txt, .docx, .html, .rtf, .md[/bold red]")
        return []

def _extract_content_epub(file_path, console):
    """Extract content from EPUB using epr's cleaner approach"""
    try:
        zip_archive = zipfile.ZipFile(file_path, 'r')
    except Exception as e:
        console.print(f"[bold red]Error: Failed to open EPUB file as ZIP: {e}[/bold red]")
        return []
    
    try:
        # Read container.xml to find OPF file location
        container_data = zip_archive.read('META-INF/container.xml')
        container_root = ET.fromstring(container_data)
        
        # Find the rootfile element
        rootfile_elem = container_root.find('.//{*}rootfile')
        if rootfile_elem is None:
            rootfile_elem = container_root.find('.//rootfile')
        
        if rootfile_elem is None:
            console.print("[bold red]Error: Could not find rootfile in container.xml[/bold red]")
            zip_archive.close()
            return []
            
        opf_path = rootfile_elem.get('full-path')
        if not opf_path:
            console.print("[bold red]Error: No full-path attribute in rootfile[/bold red]")
            zip_archive.close()
            return []
        
        # Normalize paths
        opf_path = opf_path.replace('\\', '/')
        opf_dir = os.path.dirname(opf_path) + "/" if os.path.dirname(opf_path) != "" else ""
        
        # Read and parse OPF file
        try:
            opf_data = zip_archive.read(opf_path)
        except KeyError:
            console.print(f"[bold red]Error: OPF file not found at {opf_path}[/bold red]")
            zip_archive.close()
            return []
            
        opf_root = ET.fromstring(opf_data)
        
        # Build manifest and spine
        NS = {"OPF": "http://www.idpf.org/2007/opf"}
        
        manifest = {}
        try:
            for item in opf_root.findall(".//OPF:manifest/OPF:item", NS):
                if item.get("media-type") != "application/x-dtbncx+xml" and item.get("properties") != "nav":
                    manifest[item.get("id")] = opf_dir + unquote(item.get("href"))
        except:
            # Fallback without namespace
            for item in opf_root.findall(".//manifest/item"):
                if item.get("media-type") != "application/x-dtbncx+xml" and item.get("properties") != "nav":
                    manifest[item.get("id")] = opf_dir + unquote(item.get("href"))
        
        # Get spine order
        contents = []
        try:
            spine_items = opf_root.findall(".//OPF:spine/OPF:itemref", NS)
        except:
            spine_items = opf_root.findall(".//spine/itemref")
            
        for spine_item in spine_items:
            item_id = spine_item.get("idref")
            if item_id in manifest:
                contents.append(manifest[item_id])
        
        if not contents:
            console.print("[bold red]Error: No content files found in spine[/bold red]")
            zip_archive.close()
            return []
        
        # Process each content file using HTMLtoLines parser
        chapters = []
        processed_files = 0
        
        for content_path in contents:
            try:
                # Read the HTML content
                try:
                    content_data = zip_archive.read(content_path)
                    content_str = content_data.decode('utf-8', errors='ignore')
                except KeyError:
                    continue
                except UnicodeDecodeError:
                    try:
                        content_str = content_data.decode('latin-1', errors='ignore')
                    except:
                        continue
                
                # Parse with HTMLtoLines
                parser = HTMLtoLines()
                try:
                    parser.feed(content_str)
                    parser.close()
                except:
                    continue
                
                # Get clean lines
                lines = parser.get_lines()
                
                if lines:
                    chapters.append(lines)
                    processed_files += 1
                    
            except Exception as e:
                console.print(f"[yellow]Warning: Error processing {content_path}: {e}[/yellow]")
                continue
        
        zip_archive.close()
        
        return chapters
        
    except Exception as e:
        console.print(f"[bold red]Error processing EPUB structure: {e}[/bold red]")
        try:
            zip_archive.close()
        except:
            pass
        return []

def _extract_content_pdf(file_path, console):
    from . import config
    
    def is_footnote_block(block, page_height, bottom_margin):
        """
        Detect footnotes and page numbers in bottom margin of pages.
        Filters any text that starts in the bottom margin area.
        """
        x0, y0, x1, y1, text = block[:5]
        text = text.strip()
        
        # Check if block STARTS in the bottom margin (bottom bottom_margin fraction of page)
        if y0 < page_height * (1 - bottom_margin):
            return False
            
        # If we're in the bottom margin, apply additional checks
        if text:
            # Always filter very short text in bottom margin (likely page numbers)
            if len(text) < 20:
                return True
            
            # Filter text that looks like page numbers or footers
            # Page numbers: just digits, or digits with simple text
            if re.match(r'^\d+$', text):  # Just a number
                return True
            if re.match(r'^Page\s+\d+', text, re.IGNORECASE):  # "Page 123"
                return True
            if re.match(r'^\d+\s*[-–—]\s*\d+$', text):  # "123 - 456" (page ranges)
                return True
            if re.match(r'^\d+\s*/\s*\d+$', text):  # "123 / 456" (page x of y)
                return True
            
            # Filter footnote markers and footnotes
            if re.match(r'^\d+[\.\s]', text):  # "1. footnote text"
                return True
            if re.match(r'^[*†‡§¶]', text):  # Footnote symbols
                return True
            
            # Filter common footer patterns
            if len(text) < 100 and any(word in text.lower() for word in ['chapter', 'page', 'copyright', '©']):
                return True
                
        return False

    def is_header_block(block, page_height, top_margin):
        """Check if a block is in the header area"""
        block_y0 = block[1]  # Top Y coordinate of block
        return block_y0 < page_height * top_margin

    def clean_footnote_references(text):
        cleaned = re.sub(r'\[\d+\]', '', text)
        cleaned = re.sub(r'\[[a-zA-Z]\]', '', cleaned)
        cleaned = re.sub(r'[¹²³⁴⁵⁶⁷⁸⁹⁰]+', '', cleaned)
        cleaned = re.sub(r'[*†‡§¶]', '', cleaned)
        cleaned = re.sub(r'^\d+\.\s', '', cleaned)
        cleaned = ' '.join(cleaned.split())
        return cleaned



    try:
        doc = fitz.open(file_path)
    except Exception as e:
        console.print(f"[bold red]Error: Failed to open file with fitz: {e}[/bold red]")
        return []

    # Initialize filtering based on config
    
    all_paragraphs = []
    headers_filtered = 0
    footnotes_filtered = 0
    
    for page in doc:
        page_height = page.rect.height
        page_width = page.rect.width
        main_content_blocks = []
        blocks = sorted(page.get_text("blocks"), key=lambda b: b[1])

        for block in blocks:
            # Skip footnotes (if enabled)
            if config.PDF_FILTERS_ENABLED and config.PDF_FILTER_FOOTNOTES and is_footnote_block(block, page_height, config.PDF_FOOTNOTE_MARGIN):
                footnotes_filtered += 1
                continue
                
            # Skip headers by position (if enabled)
            if config.PDF_FILTERS_ENABLED and config.PDF_FILTER_HEADERS and is_header_block(block, page_height, config.PDF_HEADER_MARGIN):
                headers_filtered += 1
                continue
                

                
            main_content_blocks.append(block)

        page_text = ""
        for block in main_content_blocks:
            block_text = block[4].replace('-\n', '').replace('\n', ' ').strip()
            page_text += block_text + " "
        
        cleaned_page_text = clean_footnote_references(page_text)
        paragraphs = cleaned_page_text.split('  ')
        
        for para in paragraphs:
            if len(para.strip()) > 25:
                cleaned_para = clean_visual_text(para.strip())
                if cleaned_para and len(cleaned_para) > 10:
                    all_paragraphs.append(cleaned_para)

    doc.close() 
    
    chapters = []
    current_chapter = []
    
    for paragraph in all_paragraphs:
        if "chapter" in paragraph.lower() and len(paragraph.split()) < 10:
            if current_chapter:
                chapters.append(current_chapter)
            current_chapter = [paragraph]
        else:
            current_chapter.append(paragraph)
            
    if current_chapter:
        chapters.append(current_chapter)
        
    if not chapters:
        return [all_paragraphs]

    return chapters

# ---------------------------------------------------------------------------
# TXT 小说章节识别
# 规则与选择算法移植自 legado（https://github.com/gedoor/legado）的
# txtTocRule.json 与 TextFile.kt，用于中文/日文等网络小说的章节分段。
# ---------------------------------------------------------------------------

# 章节标题规则，按优先级排列（对应 legado asset 中默认启用的规则）。
# 注意：Python 的 \s 与 Java 一致，均不匹配全角空格 U+3000，规则中已显式包含。
TXT_TOC_RULES = [
    # 目录(去空白)：标题前需有空白/全角空格，避免把正文顶格行误判为标题
    r"(?<=[　\s])(?:序章|楔子|正文(?!完|结)|终章|后记|尾声|番外|第\s{0,4}[\d〇零一二两三四五六七八九十百千万壹贰叁肆伍陆柒捌玖拾佰仟]+?\s{0,4}(?:章|节(?!课)|卷|集(?![合和]))).{0,30}$",
    # 目录：标准“第X章/卷/节”标题，最多 4 个缩进
    r"^[ 　\t]{0,4}(?:序章|楔子|正文(?!完|结)|终章|后记|尾声|番外|第\s{0,4}[\d〇零一二两三四五六七八九十百千万壹贰叁肆伍陆柒捌玖拾佰仟]+?\s{0,4}(?:章|节(?!课)|卷|集(?![合和])|部(?![分赛游])|篇(?!张))).{0,30}$",
    # 数字 分隔符 标题名称："1、xxx"
    r"^[ 　\t]{0,4}\d{1,5}[:：,.， 、_—\-].{1,30}$",
    # 大写数字 分隔符 标题名称："一、xxx"
    r"^[ 　\t]{0,4}(?:序章|楔子|正文(?!完|结)|终章|后记|尾声|番外|[零一二两三四五六七八九十百千万壹贰叁肆伍陆柒捌玖拾佰仟]{1,8}章?)[ 、_—\-].{1,30}$",
    # 正文 标题/序号
    r"^[ 　\t]{0,4}正文[ 　]{1,4}.{0,20}$",
    # Chapter/Section/Part/Episode 序号 标题
    r"^[ 　\t]{0,4}(?:[Cc]hapter|[Ss]ection|[Pp]art|ＰＡＲＴ|[Nn][oO][.、]|[Ee]pisode|(?:内容|文章)?简介|文案|前言|序章|楔子|正文(?!完|结)|终章|后记|尾声|番外)\s{0,4}\d{1,4}.{0,30}$",
    # 特殊符号 序号 标题："【第一章 xxx"
    r"(?<=[\s　])[【〔〖「『〈［\[](?:第|[Cc]hapter)[\d零一二两三四五六七八九十百千万壹贰叁肆伍陆柒捌玖拾佰仟]{1,10}[章节].{0,20}$",
    # 特殊符号 标题(单个)："☆、xxx"（原规则用可变宽 lookbehind，此处改写为行首形式）
    r"^[ 　\t]{0,4}(?:[☆★✦✧].{1,30}|(?:内容|文章)?简介|文案|前言|序章|楔子|正文(?!完|结)|终章|后记|尾声|番外)[ 　]{0,4}$",
    # 章/卷 序号 标题："卷五 开源盛世"
    r"^[ \t　]{0,4}(?:(?:内容|文章)?简介|文案|前言|序章|楔子|正文(?!完|结)|终章|后记|尾声|番外|[卷章][\d零一二两三四五六七八九十百千万壹贰叁肆伍陆柒捌玖拾佰仟]{1,8})[ 　]{0,4}.{0,30}$",
    # 书名 括号 序号："标题(12)"
    r"^[一-龥]{1,20}[ 　\t]{0,4}[(（][\d〇零一二两三四五六七八九十百千万壹贰叁肆伍陆柒捌玖拾佰仟]{1,8}[)）][ 　\t]{0,4}$",
    # 书名 序号："标题12"
    r"^[一-龥]{1,20}[ 　\t]{0,4}[\d〇零一二两三四五六七八九十百千万壹贰叁肆伍陆柒捌玖拾佰仟]{1,8}[ 　\t]{0,4}$",
    # 字数分割 分节阅读："分节阅读"、"第一页"（原规则用可变宽 lookbehind，此处改写为行首形式）
    r"^[ 　\t]{0,4}(?:.{0,15}分[页节章段]阅读[-_ ]|第\s{0,4}[\d零一二两三四五六七八九十百千万]{1,6}\s{0,4}[页节]).{0,30}$",
]

# 规则选择时采样的最大字符数
_TOC_RULE_SAMPLE_SIZE = 500 * 1024
# 匹配到的标题前内容超过该长度才计为有效章节（否则视为卷/误报）
_TOC_VALID_CHAPTER_LENGTH = 1000
# 匹配到的标题前内容不足该长度计为一次误报
_TOC_ERROR_CHAPTER_LENGTH = 100
# 某条规则的有效章节数超过该值即认为足够好，不再评估后续规则
_TOC_GOOD_ENOUGH = 70
# 候选规则需比当前最优规则多出该数量的有效章节才能胜出
_TOC_RULE_OVERSHOOT = 2
# 无标题规则时按字数兜底分章的长度
_TOC_FALLBACK_CHAPTER_CHARS = 10 * 1024
# 判定“每行即一段”的最大平均行长（中英文小说正文行通常不超过该值）
_AVG_LINE_AS_PARAGRAPH_LIMIT = 120
# 单行超长时按句子拆段的基准长度
_SENTENCE_PARAGRAPH_CHARS = 200


def _pick_toc_rule(sample):
    """从规则集中选出最适合当前文本的章节标题规则（移植自 legado getTocRule）。

    评估标准：标题前有足够长正文（>1000 字符）算一次有效章节，标题前内容
    不足 100 字符算一次误报；有效章节数需 >= 误报数*3，且比当前最优规则
    多出 _TOC_RULE_OVERSHOOT 个才算胜出。
    """
    best_count = -1
    best_rule = None
    for rule in TXT_TOC_RULES:
        pattern = re.compile(rule, re.MULTILINE)
        cs_num = 0
        num_e = 0
        start = 0
        for m in pattern.finditer(sample):
            content_length = m.start() - start
            if start == 0 or content_length > _TOC_VALID_CHAPTER_LENGTH:
                cs_num += 1
                start = m.end()
            elif content_length < _TOC_ERROR_CHAPTER_LENGTH:
                num_e += 1
        if cs_num >= num_e * 3 and cs_num > best_count + _TOC_RULE_OVERSHOOT:
            best_count = cs_num
            best_rule = rule
            if best_count > _TOC_GOOD_ENOUGH:
                break
    return best_rule


def _collect_title_lines(content, rule):
    """返回规则在原文中匹配到的标题所在行号（0 基，按原文行计算）。"""
    line_starts = []
    pos = 0
    for line in content.split('\n'):
        line_starts.append(pos)
        pos += len(line) + 1

    import bisect
    title_lines = set()
    for m in re.finditer(rule, content, re.MULTILINE):
        title_lines.add(bisect.bisect_right(line_starts, m.start()) - 1)
    return title_lines


def _split_txt_by_rule(content, doc_lines, rule):
    """按标题规则把（行号标注的）正文行切分为章节列表。

    doc_lines 为 [(原文行号, 清洗后文本), ...]。标题行作为该章节的第一个
    段落保留，便于 UI 展示与 TTS 朗读。第一个标题之前的正文作为“前言”章节。
    """
    title_lines = _collect_title_lines(content, rule)
    if not title_lines:
        return None

    first_title = min(title_lines)
    chapters = []
    current = []
    for line_no, text in doc_lines:
        if line_no in title_lines:
            if current:
                chapters.append(current)
            current = [text]
        elif line_no < first_title:
            # 前言正文，稍后单独成章，避免与章节内容重复
            continue
        elif text and len(text) > 3:
            current.append(text)
    if current:
        chapters.append(current)

    # 前言：第一个标题前的正文独立成章（与 legado 的序章处理一致）
    intro = [text for line_no, text in doc_lines if line_no < first_title and text]
    if intro:
        chapters.insert(0, intro)
    return chapters


def _split_txt_by_size(lines, max_chars=_TOC_FALLBACK_CHAPTER_CHARS):
    """无标题规则时的兜底：按字数把正文切分为若干章。"""
    chapters = []
    current = []
    total = 0
    for line in lines:
        current.append(line)
        total += len(line)
        if total >= max_chars:
            chapters.append(current)
            current = []
            total = 0
    if current:
        chapters.append(current)
    return chapters


def _split_long_text_into_paragraphs(text, max_chars=_SENTENCE_PARAGRAPH_CHARS):
    """整段无换行的超长文本：按句子标点拆分为多个段落。"""
    parts = re.split(r'(?<=[。！？!?；;])', text)
    paragraphs = []
    buf = ""
    for part in parts:
        part = part.strip()
        if not part:
            continue
        buf += part
        if len(buf) >= max_chars:
            paragraphs.append(buf)
            buf = ""
    if buf:
        paragraphs.append(buf)
    return paragraphs


def _read_text_with_encoding_detect(file_path, console):
    """读取文本文件，自动检测编码（UTF-8 -> GB18030 -> Latin-1）。

    GB18030 是 GBK/GB2312 的超集，覆盖国内小说 TXT 的常见编码。
    """
    try:
        with open(file_path, 'rb') as f:
            raw = f.read()
    except Exception as e:
        console.print(f"[bold red]Error: Failed to read TXT file: {e}[/bold red]")
        return None

    for encoding in ('utf-8-sig', 'gb18030'):
        try:
            return raw.decode(encoding)
        except UnicodeDecodeError:
            continue
    return raw.decode('latin-1', errors='replace')


def _extract_content_txt(file_path, console):
    content = _read_text_with_encoding_detect(file_path, console)
    if content is None:
        return []

    # Normalize newlines to handle mixed cases
    content = content.replace('\r\n', '\n').replace('\r', '\n')

    raw_lines = content.split('\n')
    if not any(l.strip() for l in raw_lines):
        console.print("[bold red]No text content found in the TXT file.[/bold red]")
        return []

    # 段落切分策略：
    # 1. 若文本几乎不换行（单行超长），按句子标点拆成段落，保证 h/l 可逐段跳转；
    # 2. 若以中文为主且正文行不长（典型小说格式：一行一段），按行切分段落，
    #    避免空行分段把整章内容合并成一个大段落（h/l 整章跳转的根因）；
    # 3. 其余情况（英文排版文本等）保留空行分段逻辑。
    cjk_lines = [l for l in raw_lines if re.search(r'[\u4e00-\u9fff]', l)]
    is_chinese = len(cjk_lines) / len(raw_lines) > 0.5
    cjk_avg_len = sum(len(l) for l in cjk_lines) / max(1, len(cjk_lines))

    # 构建 (原文行号, 清洗后文本) 列表，空行剔除；
    # 短行不在此处过滤，避免滤掉“尾声”“楔子”等两字章节标题
    doc_lines = []
    for i, line in enumerate(raw_lines):
        text = clean_visual_text(line.strip())
        if text:
            doc_lines.append((i, text))

    use_title_rule = True
    if len(doc_lines) <= 1 and len(content) > _SENTENCE_PARAGRAPH_CHARS:
        # 几乎没有换行：按句子拆段，此时行号与原文不再对应，无法用标题规则切章
        sentence_paras = _split_long_text_into_paragraphs(doc_lines[0][1])
        doc_lines = [(i, p) for i, p in enumerate(sentence_paras)]
        doc_lines = [(i, t) for i, t in doc_lines if t and len(t) > 3]
        use_title_rule = False
    elif is_chinese and cjk_avg_len <= _AVG_LINE_AS_PARAGRAPH_LIMIT:
        pass  # 行级段落：doc_lines 本身即“一行一段”
    else:
        # 空行分段：行号连续的行合并为一段，行号跳变（隔了空行）则新起一段
        merged = []
        cur = []
        prev = None
        for line_no, text in doc_lines:
            if prev is not None and line_no == prev + 1:
                cur.append(text)
            else:
                if cur:
                    merged.append(" ".join(cur))
                cur = [text]
            prev = line_no
        if cur:
            merged.append(" ".join(cur))
        doc_lines = [(i, t) for i, t in enumerate(merged)]
        use_title_rule = False  # 合并后行号不再对应原文，交由字数兜底分章

    if not doc_lines:
        console.print("[bold red]No text content found in the TXT file.[/bold red]")
        return []

    # 章节识别：先尝试标题规则，匹配不到足够章节时按字数兜底
    if use_title_rule:
        rule = _pick_toc_rule(content[:_TOC_RULE_SAMPLE_SIZE])
        if rule:
            chapters = _split_txt_by_rule(content, doc_lines, rule)
            if chapters:
                return chapters

    paragraphs = [text for _, text in doc_lines]
    return _split_txt_by_size(paragraphs)


def _extract_content_docx(file_path, console):
    """
    Extracts content from a .docx file, preserving paragraphs.
    It uses the 'python-docx' library.
    """
    try:
        doc = Document(file_path)
        full_text = "\n".join([para.text for para in doc.paragraphs if para.text and not para.text.isspace()])
        paragraphs = [clean_visual_text(p.strip()) for p in full_text.split('\n') if p.strip()]
        paragraphs = [p for p in paragraphs if p and len(p) > 3]
        
        return [paragraphs]
    except Exception as e:
        console.print(f"[bold red]Error: Failed to read DOCX file: {e}[/bold red]")
        return []
        
def _extract_content_rtf(file_path, console):
    """
    Extracts content from an .rtf file using the 'striprtf' library.
    """
    try:
        with open(file_path, 'r', encoding='utf-8', errors='ignore') as f:
            rtf_content = f.read()
            
        text_content = rtf_to_text(rtf_content, errors="ignore")
        
        text_content = text_content.replace('\r\n', '\n').replace('\r', '\n')
        
        lines = [clean_visual_text(line.strip()) for line in text_content.split('\n') if line.strip()]
        lines = [line for line in lines if line and len(line) > 3]

        return [lines]
    except Exception as e:
        console.print(f"[bold red]Error: Failed to parse RTF file: {e}[/bold red]")
        return []


        
def _extract_content_md(file_path, console):
    try:
        with open(file_path, 'r', encoding='utf-8') as f:
            md_content = f.read()
    except UnicodeDecodeError:
        try:
            with open(file_path, 'r', encoding='latin-1') as f:
                md_content = f.read()
        except Exception as e:
            console.print(f"[bold red]Error: Failed to read Markdown file: {e}[/bold red]")
            return []
    except Exception as e:
        console.print(f"[bold red]Error: Failed to read Markdown file: {e}[/bold red]")
        return []

    try:
        # Use enhanced raw markdown parsing for better structure preservation
        return [_parse_raw_markdown(md_content)]
        
    except Exception as e:
        # If raw parsing fails, try HTML conversion as fallback
        try:
            html_content = markdown.markdown(md_content, extensions=['codehilite', 'fenced_code'])
            parser = HTMLtoLines()
            parser.feed(html_content)
            parser.close()
            
            lines = parser.get_lines()
            if lines:
                return [lines]
            else:
                console.print(f"[bold red]Error parsing Markdown content: {e}[/bold red]")
                return []
                
        except Exception as e2:
            console.print(f"[bold red]Error parsing Markdown content: {e2}[/bold red]")
            return []


def _parse_raw_markdown(md_content):
    """Parse raw markdown content while preserving structure"""
    lines = md_content.split('\n')
    result = []
    in_code_block = False
    code_fence = None
    
    i = 0
    while i < len(lines):
        line = lines[i].rstrip()
        
        # Handle code blocks
        if line.startswith('```') or line.startswith('~~~'):
            if not in_code_block:
                # Starting a code block
                in_code_block = True
                code_fence = line[:3]
                result.append("")  # Add spacing before code block
                # Extract language if specified
                lang = line[3:].strip()
                if lang:
                    result.append(f"Code ({lang}):")
                else:
                    result.append("Code:")
                i += 1
                continue
            elif line.startswith(code_fence):
                # Ending a code block
                in_code_block = False
                code_fence = None
                result.append("")  # Add spacing after code block
                i += 1
                continue
        
        if in_code_block:
            # Inside code block - preserve indentation and mark as code to skip cleaning
            result.append(f"__CODE_BLOCK__    {line}")
        elif line.startswith('#'):
            # Headers
            level = len(line) - len(line.lstrip('#'))
            header_text = line.lstrip('# ').strip()
            if header_text:
                result.append("")
                result.append(header_text)
                result.append("")
        elif line.startswith(('- ', '* ', '+ ')) or re.match(r'^\d+\.\s', line):
            # List items
            if line.startswith(('- ', '* ', '+ ')):
                list_text = line[2:].strip()
                result.append(f"• {list_text}")
            else:
                # Numbered list
                list_text = re.sub(r'^\d+\.\s', '', line).strip()
                result.append(f"• {list_text}")
        elif re.match(r'^\s+(-|\*|\+|\d+\.)\s+', line):
            # Indented list items (should be cleaned, not treated as code)
            # Extract the list content after the indentation and list marker
            indented_list_text = re.sub(r'^\s+(-|\*|\+|\d+\.)\s+', '', line)
            result.append(f"    • {indented_list_text}")
        elif line.startswith('    ') or line.startswith('\t'):
            # Indented content (code blocks) - preserve formatting
            result.append(f"__CODE_BLOCK__    {line.strip()}")
        elif line.startswith('>'):
            # Blockquotes
            quote_text = line.lstrip('> ').strip()
            if quote_text:
                result.append(f"    {quote_text}")
        elif line.strip() == '':
            # Empty lines - preserve but limit consecutive ones
            if result and result[-1] != '':
                result.append('')
        else:
            # Regular paragraphs
            if line.strip():
                result.append(line.strip())
        
        i += 1
    
    # Apply global visual text cleaning to all lines
    result = [clean_visual_text(line) if line.strip() else line for line in result]
    
    # Clean up excessive empty lines
    clean_result = []
    empty_count = 0
    for line in result:
        if line == '':
            empty_count += 1
            if empty_count <= 2:
                clean_result.append(line)
        else:
            empty_count = 0
            clean_result.append(line)
    
    return [line for line in clean_result if line or len(clean_result) < 100]  # Keep empty lines for short docs



def _extract_content_html(file_path, console):
    try:
        with open(file_path, 'r', encoding='utf-8') as f:
            content = f.read()
    except UnicodeDecodeError:
        try:
            with open(file_path, 'r', encoding='latin-1') as f:
                content = f.read()
        except Exception as e:
            console.print(f"[bold red]Error: Failed to read HTML file: {e}[/bold red]")
            return []
    except Exception as e:
        console.print(f"[bold red]Error: Failed to read HTML file: {e}[/bold red]")
        return []

    try:
        parser = HTMLtoLines()
        parser.feed(content)
        parser.close()
        
        # Get clean lines
        lines = parser.get_lines()
        
        if not lines:
            console.print("[bold red]No text content found in the HTML file.[/bold red]")
            return []

        return [lines]
    except Exception as e:
        console.print(f"[bold red]Error: Failed to parse HTML file: {e}[/bold red]")
        return []
