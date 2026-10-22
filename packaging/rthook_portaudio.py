# PyInstaller runtime hook (Linux): point sounddevice at the PortAudio the
# app ships with. The library is built without JACK, so it only needs ALSA,
# which every desktop has; a distro's libportaudio2 needs JACK installed too.
import ctypes.util
import os
import sys

_bundled = os.path.join(sys._MEIPASS, "libportaudio.so.2")
if os.path.exists(_bundled):
    _find_library = ctypes.util.find_library

    def _find(name):
        return _bundled if name == "portaudio" else _find_library(name)

    ctypes.util.find_library = _find
