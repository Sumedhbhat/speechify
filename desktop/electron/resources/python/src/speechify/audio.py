"""File-only training audio helpers. No microphone, playback or UI code."""
import wave
import numpy as np
SAMPLE_RATE = 16000

def load_wav(path):
    with wave.open(str(path), 'rb') as f:
        if f.getframerate() != SAMPLE_RATE or f.getnchannels() != 1 or f.getsampwidth() != 2:
            raise ValueError('Expected mono 16kHz PCM16 WAV')
        return np.frombuffer(f.readframes(f.getnframes()), dtype=np.int16).astype(np.float32) / 32768

def save_wav(path, audio):
    with wave.open(str(path), 'wb') as f:
        f.setnchannels(1)
        f.setsampwidth(2)
        f.setframerate(SAMPLE_RATE)
        f.writeframes((np.clip(audio, -1, 1) * 32767).astype(np.int16).tobytes())
