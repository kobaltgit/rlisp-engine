#!/usr/bin/env python3
"""Собирает GIF и filmstrip из кадров захвата.

Использование:
  python3 scripts/make_gif.py [glob_кадров] [выход.gif] [выход_strip.png]
По умолчанию — 2D-демо: demo/frames → demo/survival.gif + demo/filmstrip.png.
Для 3D: python3 scripts/make_gif.py demo/frames3d demo/orbital.gif demo/filmstrip3d.png
"""
import glob
import sys

from PIL import Image

frames_glob = sys.argv[1] if len(sys.argv) > 1 else "demo/frames/frame_*.png"
gif_out = sys.argv[2] if len(sys.argv) > 2 else "demo/survival.gif"
strip_out = sys.argv[3] if len(sys.argv) > 3 else "demo/filmstrip.png"

files = sorted(glob.glob(frames_glob))
if not files:
    sys.exit(f"нет кадров: {frames_glob}")

frames = [Image.open(f).convert("RGB") for f in files]
frames[0].save(gif_out, save_all=True, append_images=frames[1:],
               duration=200, loop=0)
print(f"{gif_out}: {len(frames)} кадров")

n = 9
idx = [min(int(len(files) * i / n), len(files) - 1) for i in range(n)]
w, h = frames[0].size
strip = Image.new("RGB", (w * 3, h * 3), (10, 10, 18))
for k, i in enumerate(idx):
    strip.paste(frames[i], ((k % 3) * w, (k // 3) * h))
strip.save(strip_out)
print(f"{strip_out}: кадры", [files[i] for i in idx][-3:], "…")
