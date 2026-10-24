"""Writes the app icon (src/speechify/icon.py) in each platform's format.

    uv run --group dev packaging/make_icons.py packaging/build

Writes icon.png (256 px, Linux), icon.ico (Windows) and icon.icns (macOS).
"""

import sys
from pathlib import Path

from PIL import Image

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))
from speechify.icon import draw  # noqa: E402


def main(out: Path) -> None:
    out.mkdir(parents=True, exist_ok=True)
    icon = draw()
    icon.resize((256, 256), Image.LANCZOS).save(out / "icon.png")
    icon.save(out / "icon.ico", sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)])
    icon.save(out / "icon.icns")


if __name__ == "__main__":
    main(Path(sys.argv[1]))
