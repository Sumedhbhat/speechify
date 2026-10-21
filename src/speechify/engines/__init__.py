"""Speech recognition engines.

Each model runs in a worker process inside its backend's own environment
(see catalog.py), so only the selected model is loaded and conflicting
dependencies never meet. Nothing here imports a heavy library at module
level; the app process stays small.
"""
