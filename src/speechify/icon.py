"""The app icon: a white microphone on an indigo rounded square.

Drawn with Pillow rather than shipped as a file, so the tray icon and the
installers' icons (packaging/make_icons.py) are the same picture.
"""

from __future__ import annotations

BACKGROUND = (79, 70, 229)  # indigo
FOREGROUND = (255, 255, 255)
LISTENING = (229, 72, 77)  # the tray icon while the talk key is held


def draw(size: int = 1024, background: tuple[int, int, int] = BACKGROUND):
    from PIL import Image, ImageDraw

    scale = 4  # draw large and shrink, for smooth edges
    s = size * scale
    img = Image.new("RGBA", (s, s), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    margin = s * 0.06
    d.rounded_rectangle((margin, margin, s - margin, s - margin), radius=s * 0.22, fill=background)

    cx = s / 2
    w = s * 0.08  # stroke width
    # The microphone capsule.
    d.rounded_rectangle((cx - s * 0.12, s * 0.2, cx + s * 0.12, s * 0.56), radius=s * 0.12, fill=FOREGROUND)
    # The holder: a U around the capsule's lower half.
    r = s * 0.2
    top = s * 0.33
    d.arc((cx - r, top, cx + r, top + 2 * r), start=0, end=180, fill=FOREGROUND, width=int(w))
    # Stem and base.
    d.rectangle((cx - w / 2, top + 2 * r - w / 2, cx + w / 2, s * 0.8), fill=FOREGROUND)
    d.rounded_rectangle((cx - s * 0.15, s * 0.77, cx + s * 0.15, s * 0.77 + w), radius=w / 2, fill=FOREGROUND)
    return img.resize((size, size), Image.LANCZOS)
