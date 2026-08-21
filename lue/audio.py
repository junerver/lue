import asyncio
import os
import re
import subprocess
import logging
from . import config, content_parser

# This pattern is used to both clean text for TTS and detect sentence fragments.
ABBREVIATION_PATTERN = r'\b(Mr|Mrs|Ms|Dr|Prof|Rev|Hon|Jr|Sr|Cpl|Sgt|Gen|Col|Capt|Lt|Pvt|vs|viz|Co|Inc|Ltd|Corp|St|Ave|Blvd)\.'
INITIAL_PATTERN = r'\b([A-Z])\.(?=\s[A-Z])'


# Word mapping functionality moved to timing_calculator.py
# Import it here for backward compatibility
from .timing_calculator import create_word_mapping as _create_word_mapping


def clean_tts_text(text: str) -> str:
    """
    Removes periods from specific English abbreviations and single initials
    to prevent unnatural pauses in TTS engines. Also removes loose punctuation
    marks that are not connected to any word.
    """
    # Remove periods from abbreviations and initials
    text = re.sub(ABBREVIATION_PATTERN, r'\1', text)
    text = re.sub(INITIAL_PATTERN, r'\1 ', text)
    
    # Remove loose punctuation marks that are standalone (not connected to words)
    # This pattern matches punctuation that is surrounded by whitespace or at string boundaries
    text = re.sub(r'(?:^|\s)[.,:;!?]+(?=\s|$)', ' ', text)
    
    # Remove standalone dashes that are followed by quotation marks
    # This prevents TTS engines from reading "-" as "dash" in cases like: -" 
    text = re.sub(r'(?:^|\s)-(?=")', ' ', text)
    
    # Clean up any extra whitespace that might result from removing punctuation
    text = re.sub(r'\s+', ' ', text).strip()
    
    return text

async def stop_and_clear_audio(reader):
    """Stop audio playback and clear the audio queue."""
    tasks_to_cancel = []
    for task in [reader.producer_task, reader.player_task]:
        if task and not task.done():
            task.cancel()
            tasks_to_cancel.append(task)
    if tasks_to_cancel:
        await asyncio.gather(*tasks_to_cancel, return_exceptions=True)
    
    reader.producer_task = None
    reader.player_task = None
    
    processes_to_kill = reader.playback_processes.copy()
    reader.playback_processes.clear()
    for process in processes_to_kill:
        try:
            if process.returncode is None:
                process.terminate()
                try: await asyncio.wait_for(process.wait(), timeout=0.2)
                except asyncio.TimeoutError:
                    process.kill()
                    await asyncio.wait_for(process.wait(), timeout=0.1)
        except (ProcessLookupError, AttributeError, asyncio.TimeoutError): pass
    
    try:
        pkill_proc = await asyncio.create_subprocess_exec('pkill', '-9', '-f', 'ffplay.*buffer_', stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        await asyncio.wait_for(pkill_proc.wait(), timeout=0.3)
    except (FileNotFoundError, asyncio.TimeoutError): pass
    
    while not reader.audio_queue.empty():
        try:
            reader.audio_queue.get_nowait()
            reader.audio_queue.task_done()
        except asyncio.QueueEmpty: break
    
    await asyncio.sleep(0.1)
    
    # More aggressive cleanup with longer delays for file system operations
    for buf_base in config.AUDIO_BUFFERS:
        for ext in ['.mp3', '.wav']:
            buf = f"{buf_base}{ext}"
            for attempt in range(5):  # Increased attempts
                try:
                    if os.path.exists(buf): 
                        os.remove(buf)
                    break
                except OSError:
                    if attempt < 4: 
                        await asyncio.sleep(0.1)  # Longer delay
            

    
    await asyncio.sleep(0.2)  # Longer final delay


        
async def get_audio_duration(file_path):
    """Get the duration of an audio file."""
    command = ['ffprobe', '-v', 'error', '-show_entries', 'format=duration', '-of', 'default=noprint_wrappers=1:nokey=1', file_path]
    process = await asyncio.create_subprocess_exec(*command, stdout=asyncio.subprocess.PIPE, stderr=subprocess.DEVNULL)
    stdout, _ = await process.communicate()
    if process.returncode != 0: return None
    try: return float(stdout.decode().strip())
    except (ValueError, TypeError): return None

async def play_from_current_position(reader):
    """Start the audio producer and player loops."""
    # tts_ready: TTS 模型后台初始化完成前不启动播放（初始化完成后会重新调用）
    if not reader.is_paused and reader.running and reader.tts_model and getattr(reader, 'tts_ready', True):
        # Cancel existing tasks and wait for them to complete
        for task in [reader.producer_task, reader.player_task]:
            if task and not task.done():
                task.cancel()
                try: 
                    await asyncio.wait_for(task, timeout=2.0)
                except (asyncio.CancelledError, asyncio.TimeoutError): 
                    pass
        
        # Ensure tasks are properly cleaned up
        reader.producer_task = None
        reader.player_task = None
        
        # Small delay to ensure cleanup is complete
        await asyncio.sleep(0.05)
        
        reader.producer_task = asyncio.create_task(_producer_loop(reader))
        reader.player_task = asyncio.create_task(_player_loop(reader))

async def _producer_loop(reader):
    """Producer loop to generate audio files."""
    if not reader.tts_model or not reader.tts_model.initialized:
        try: await asyncio.wait_for(reader.audio_queue.put(None), timeout=0.5)
        except (asyncio.TimeoutError, asyncio.CancelledError): pass
        return

    producer_pos = (reader.chapter_idx, reader.paragraph_idx, reader.sentence_idx)
    buffer_index = 0
    try:
        while reader.running:
            if reader.audio_queue.full():
                await asyncio.sleep(0.1)
                continue
            try:
                c, p, s = producer_pos
                sentences = content_parser.split_into_sentences(reader.chapters[c][p])
                text = sentences[s]
            except IndexError: break
            if not text or not text.strip():
                next_pos = reader._advance_position(producer_pos, wrap=False)
                if not next_pos: break
                producer_pos = next_pos
                continue

            # --- Start of fragment merging logic ---
            merged = False
            # Heuristic: if a "sentence" is just an abbreviation, it might be a fragment.
            # We check if the entire text matches common abbreviation patterns.
            is_abbrev_fragment = re.fullmatch(ABBREVIATION_PATTERN, text.strip())

            if is_abbrev_fragment and s + 1 < len(sentences):
                text += " " + sentences[s+1]
                merged = True
            # --- End of fragment merging logic ---

            # Preserve original text for UI display and timing calculation
            original_text = text
            
            output_format = reader.tts_model.output_format
            output_filename = f"{config.AUDIO_BUFFERS[buffer_index]}.{output_format}"
            
            try:
                if not reader.running: break
                
                for attempt in range(3):
                    try:
                        if os.path.exists(output_filename): os.remove(output_filename)
                        break
                    except OSError:
                        if attempt < 2: await asyncio.sleep(0.05)
                
                # Create sanitized version for TTS
                sanitized_text = content_parser.sanitize_text_for_tts(original_text)
                
                timing_info = None
                
                # Use the timing-aware method if available
                if hasattr(reader.tts_model, 'generate_audio_with_timing'):
                    try:
                        timing_info = await reader.tts_model.generate_audio_with_timing(sanitized_text, output_filename)
                    except Exception as e:
                        # If timing generation fails, fall back to generating without it
                        logging.error(f"TTS timing generation failed for text '{original_text[:50]}...' (sanitized: '{sanitized_text[:50]}...'): {e}")
                        await reader.tts_model.generate_audio(sanitized_text, output_filename)
                else:
                    # Fallback to regular method
                    await reader.tts_model.generate_audio(sanitized_text, output_filename)

                # Always get the actual duration from the file
                duration = await get_audio_duration(output_filename)
                
                if not reader.running: break
                
                # If no timing info was generated, create a fallback structure
                # Pass original_text to timing calculator for proper word mapping
                if timing_info is None:
                    from .timing_calculator import process_tts_timing_data
                    timing_info = process_tts_timing_data(original_text, [], duration)
                
                await asyncio.wait_for(reader.audio_queue.put((output_filename, *producer_pos, duration, timing_info)), timeout=1.0)
                
                next_pos = reader._advance_position(producer_pos, wrap=False)
                if merged:
                    # If we merged two sentences, we must advance the position an extra time.
                    if next_pos:
                        next_pos = reader._advance_position(next_pos, wrap=False)

                if not next_pos: break
                producer_pos = next_pos
                buffer_index = (buffer_index + 1) % len(config.AUDIO_BUFFERS)
            except asyncio.CancelledError: break
            except Exception as e:
                if reader.running:
                    # Include both original and sanitized text in error logging
                    try:
                        sanitized_for_log = content_parser.sanitize_text_for_tts(original_text) if 'original_text' in locals() else 'N/A'
                        original_for_log = original_text if 'original_text' in locals() else 'N/A'
                        logging.error(f"TTS Error in producer: {e}\nOriginal text: '{original_for_log[:100]}...'\nSanitized text: '{sanitized_for_log[:100]}...'", exc_info=True)
                    except:
                        logging.error(f"TTS Error in producer: {e}", exc_info=True)
                    await asyncio.sleep(2)
                continue
    except asyncio.CancelledError: pass
    finally:
        try: await asyncio.wait_for(reader.audio_queue.put(None), timeout=0.5)
        except (asyncio.TimeoutError, asyncio.CancelledError): pass

async def _player_loop(reader):
    """Player loop to play audio files."""
    try:
        while reader.running:
            try:
                item = await asyncio.wait_for(reader.audio_queue.get(), timeout=1.0)
                if item is None:
                    reader.audio_queue.task_done()
                    if reader.active_playback_tasks:
                        await asyncio.gather(*reader.active_playback_tasks, return_exceptions=True)
                    reader.playback_finished_event.set()
                    break
                # Unpack the queue item
                audio_file, c, p, s, duration, timing_data = item
                if isinstance(timing_data, dict):
                    timing_info = timing_data
                else:
                    # Old format, timing_data is word_timings
                    timing_info = {"word_timings": timing_data, "speech_duration": duration, "total_duration": duration}

                word_timings = timing_info.get("word_timings", [])
                
                if not os.path.exists(audio_file):
                    reader.audio_queue.task_done()
                    continue
                if duration is None or duration <= 0:
                    reader.audio_queue.task_done()
                    continue
                try:
                    # Post a command to the main loop to handle the state transition atomically
                    reader.loop.call_soon_threadsafe(
                        reader._post_command_sync,
                        ('_new_sentence_started', (c, p, s, duration, timing_data))
                    )
                except RuntimeError:
                    reader.audio_queue.task_done()
                    break
                try:
                    # Build ffplay command with speed control using atempo filter
                    cmd = ['ffplay', '-nodisp', '-autoexit', '-loglevel', 'error']
                    
                    # Add atempo filter if speed is not 1.0
                    if abs(reader.playback_speed - 1.0) > 0.01:
                        # atempo filter has limitations: must be between 0.5 and 2.0
                        # For speeds outside this range, we chain multiple atempo filters
                        speed = reader.playback_speed
                        filters = []
                        
                        while speed > 2.0:
                            filters.append('atempo=2.0')
                            speed /= 2.0
                        while speed < 0.5:
                            filters.append('atempo=0.5')
                            speed /= 0.5
                        if abs(speed - 1.0) > 0.01:
                            filters.append(f'atempo={speed:.3f}')
                        
                        if filters:
                            filter_chain = ','.join(filters)
                            cmd.extend(['-af', filter_chain])
                    
                    cmd.append(audio_file)
                    process = await asyncio.create_subprocess_exec(*cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
                    reader.playback_processes.append(process)
                except Exception:
                    reader.audio_queue.task_done()
                    continue

                async def await_and_remove(proc, file):
                    task = asyncio.current_task()
                    try:
                        await proc.wait()
                    except Exception: pass
                    finally:
                        try:
                            if proc in reader.playback_processes: reader.playback_processes.remove(proc)
                        except ValueError: pass
                        for attempt in range(3):
                            try:
                                if os.path.exists(file): os.remove(file)
                                break
                            except OSError:
                                if attempt < 2: await asyncio.sleep(0.05)
                        if task in reader.active_playback_tasks:
                            reader.active_playback_tasks.remove(task)

                playback_task = asyncio.create_task(await_and_remove(process, audio_file))
                reader.active_playback_tasks.append(playback_task)
                
                # Calculate dynamic overlap based on playback speed
                # Overlap should decrease as speed increases, reaching 0 at 3.00x speed and beyond
                base_overlap = config.OVERLAP_SECONDS
                if reader.tts_model and hasattr(reader.tts_model, 'get_overlap_seconds'):
                    tts_overlap = reader.tts_model.get_overlap_seconds()
                    if tts_overlap is not None:
                        base_overlap = tts_overlap
                
                # Apply speed-based overlap reduction
                # At 1.0x speed: full overlap
                # At 3.00x speed and above: 0 overlap
                # Linear decrease between 1.0x and 3.0x
                speed = reader.playback_speed
                if speed >= 3.0:
                    overlap_seconds = 0.0
                else:
                    # Calculate overlap as a linear function decreasing from base_overlap to 0
                    # as speed increases from 1.0 to 3.0
                    overlap_factor = max(0.0, min(1.0, (3.0 - speed) / (3.0 - 1.0)))
                    overlap_seconds = base_overlap * overlap_factor
                
                # Adjust duration for playback speed
                actual_duration = duration / speed
                
                await asyncio.sleep(max(0.1, actual_duration - overlap_seconds))
                reader.audio_queue.task_done()
            except asyncio.TimeoutError:
                if not reader.running: break
                continue
            except asyncio.CancelledError: break
    except asyncio.CancelledError: pass
    finally:
        for process in reader.playback_processes.copy():
            try:
                if process.returncode is None: process.terminate()
            except (ProcessLookupError, AttributeError): pass