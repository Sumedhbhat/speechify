"""Hold-to-talk local dictation that learns from your corrections."""

from pathlib import Path


def _version() -> str:
    # Release builds stamp the full version (e.g. 0.2.0-nightly.57) into a
    # VERSION file; a source checkout falls back to the pyproject version.
    stamped = Path(__file__).with_name("VERSION")
    if stamped.exists():
        return stamped.read_text(encoding="utf-8").strip()
    pyproject = Path(__file__).resolve().parents[2] / "pyproject.toml"
    if pyproject.exists():
        import tomllib

        return tomllib.loads(pyproject.read_text(encoding="utf-8"))["project"]["version"]
    try:
        from importlib.metadata import version

        return version("speechify")
    except Exception:
        return "0.0.0+unknown"


__version__ = _version()
