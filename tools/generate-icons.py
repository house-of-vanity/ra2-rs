"""Regenerate native desktop icons from the original artwork (requires Pillow)."""
from pathlib import Path

from PIL import Image


directory = Path(__file__).resolve().parents[1] / "assets" / "icons"
with Image.open(directory / "ra2-rs.png") as source:
    rgba = source.convert("RGBA")
    sizes = (16, 24, 32, 48, 64, 128, 256, 512, 1024)
    images = {
        size: rgba.resize((size, size), Image.Resampling.LANCZOS)
        for size in sizes
    }
    for size in (48, 256):
        images[size].save(directory / f"ra2-rs-{size}.png")
    source.save(directory / "ra2-rs.ico", append_images=list(images.values()))
    source.save(directory / "ra2-rs.icns", append_images=list(images.values()))
