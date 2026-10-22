"""Draws the app icon and writes it in each platform's format.

    uv run --group dev packaging/make_icons.py packaging/build

Writes icon.png (256 px, Linux), icon.ico (Windows) and icon.icns (macOS).
"""

import sys
from pathlib import Path

from PIL import Image, ImageDraw

SIZE = 1024
BACKGROUND = (79, 70, 229)  # indigo
FOREGROUND = (255, 255, 255)


def draw() -> Image.Image:
    scale = 4  # draw large and shrink, for smooth edges
    s = SIZE * scale
    img = Image.new("RGBA", (s, s), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    margin = s * 0.06
    d.rounded_rectangle((margin, margin, s - margin, s - margin), radius=s * 0.22, fill=BACKGROUND)

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
    return img.resize((SIZE, SIZE), Image.LANCZOS)


def main(out: Path) -> None:
    out.mkdir(parents=True, exist_ok=True)
    icon = draw()
    icon.resize((256, 256), Image.LANCZOS).save(out / "icon.png")
    icon.save(out / "icon.ico", sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)])
    icon.save(out / "icon.icns")


if __name__ == "__main__":
    main(Path(sys.argv[1]))
