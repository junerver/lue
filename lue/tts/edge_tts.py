import os
import asyncio
import logging
from rich.console import Console

from .base import TTSBase
from .. import config

class EdgeTTS(TTSBase):
    """TTS implementation for Microsoft Edge's online TTS service."""

    @property
    def name(self) -> str:
        return "edge"

    @property
    def output_format(self) -> str:
        return "mp3"

    def __init__(self, console: Console, voice: str = None, lang: str = None):
        super().__init__(console, voice, lang)
        self.edge_tts = None
        if self.voice is None:
            self.voice = config.TTS_VOICES.get(self.name)

    async def initialize(self) -> bool:
        """Checks if the edge-tts library is available."""
        try:
            import edge_tts
            self.edge_tts = edge_tts
            self.initialized = True
            logging.info("Edge TTS model is available.")
            return True
        except ImportError:
            logging.error("'edge-tts' is not installed. Please run 'pip install edge-tts' to use this TTS model.")
            return False

    async def get_raw_timing_data(self, text: str, output_path: str):
        """
        Get raw word timing data from Edge TTS.
        
        Returns:
            List of (word, start_time, end_time) tuples with raw timing data from Edge TTS
        """
        if not self.initialized:
            raise RuntimeError("Edge TTS has not been initialized.")
        
        try:
            communicate = self.edge_tts.Communicate(text, self.voice, boundary="WordBoundary")
            
            # Collect word timing information
            word_timings = []
            audio_chunks = []
            
            async for chunk in communicate.stream():
                if chunk['type'] == 'WordBoundary':
                    # Convert from 100-nanosecond units to seconds
                    start_time = chunk['offset'] / 10000000.0
                    end_time = (chunk['offset'] + chunk['duration']) / 10000000.0
                    word_timings.append((chunk['text'], start_time, end_time))
                elif chunk['type'] == 'audio':
                    audio_chunks.append(chunk['data'])
            
            # Save audio to file
            with open(output_path, 'wb') as f:
                for chunk in audio_chunks:
                    f.write(chunk)
            
            return word_timings
            
        except Exception as e:
            logging.error(f"Edge TTS audio generation failed for text: '{text[:50]}...'", exc_info=True)
            raise e

    async def generate_audio_with_timing(self, text: str, output_path: str):
        """
        Generate audio with timing using the centralized timing calculator.
        
        This method leverages Edge TTS's precise word boundary information
        through get_raw_timing_data() and processes it with the timing calculator.
        """
        # Get raw timing data (which also generates the audio)
        raw_timings = await self.get_raw_timing_data(text, output_path)
        
        # Get actual audio duration
        try:
            from .. import audio
        except ImportError:
            import sys
            import os
            sys.path.insert(0, os.path.dirname(os.path.dirname(__file__)))
            import audio
        duration = await audio.get_audio_duration(output_path)
        
        # Process timing data using the centralized calculator
        try:
            from ..timing_calculator import process_tts_timing_data
        except ImportError:
            import sys
            import os
            sys.path.insert(0, os.path.dirname(os.path.dirname(__file__)))
            import timing_calculator
            process_tts_timing_data = timing_calculator.process_tts_timing_data
        return process_tts_timing_data(text, raw_timings, duration)

    async def generate_audio(self, text: str, output_path: str):
        """Generates audio from text using edge-tts and saves it to a file."""
        if not self.initialized:
            raise RuntimeError("Edge TTS has not been initialized.")
        try:
            communicate = self.edge_tts.Communicate(text, self.voice)
            await communicate.save(output_path)
        except Exception as e:
            logging.error(f"Edge TTS audio generation failed for text: '{text[:50]}...'", exc_info=True)
            raise e

    async def warm_up(self):
        """Warms up the TTS model by making a short request."""
        if not self.initialized:
            return

        logging.info("Warming up the Edge TTS model...")
        warmup_file = os.path.join(config.AUDIO_DATA_DIR, f".warmup_edge.{self.output_format}")
        try:
            await self.generate_audio("Ready.", warmup_file)
            logging.info("Edge TTS model is ready.")
        except Exception as e:
            logging.warning(f"Edge TTS model warm-up failed (possible network issue or invalid voice: {self.voice}): {e}", exc_info=True)
        finally:
            if os.path.exists(warmup_file):
                try:
                    os.remove(warmup_file)
                except OSError:
                    pass