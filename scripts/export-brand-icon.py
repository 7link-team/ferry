"""Derive app/browser sizes from public/brand/logo.png; preserve its RGBA artwork."""
from pathlib import Path
from PIL import Image

root = Path(__file__).resolve().parent.parent / "public" / "brand"
master = Image.open(root / "logo.png").convert("RGBA")
sizes = [16, 32, 48, 64, 128, 180, 256, 512]
frames = {size: master.resize((size, size), Image.Resampling.LANCZOS) for size in sizes}
for size, image in frames.items():
    image.save(root / f"logo-{size}.png", optimize=True)
ico_sizes = [16, 32, 48, 64, 128, 256]
master.save(root / "logo.ico", sizes=[(s, s) for s in ico_sizes], append_images=[frames[s] for s in ico_sizes])
assert Image.open(root / "logo.ico").ico.sizes() == {(s, s) for s in ico_sizes}
