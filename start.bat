@echo off
cd /d "%~dp0"
rem --frozen: run exactly what uv.lock pins, without re-resolving at startup.
uv run --frozen --no-dev speechify %*
