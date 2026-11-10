#!/usr/bin/env python3
"""Собирает physpoly.gif / filmstrip / стиллы из demo/frames_pp + PIL-сверка."""
import glob
import os

from PIL import Image

fs = sorted(glob.glob("demo/frames_pp/frame_*.png"))
assert fs, "нет кадров demo/frames_pp"
frames = [Image.open(f).convert("RGB") for f in fs]
sub = frames[::2]  # 40 -> 20 в GIF
sub[0].save("demo/physpoly.gif", save_all=True, append_images=sub[1:],
            duration=120, loop=0, optimize=True)
w, h = frames[0].size
strip = Image.new("RGB", (w * 3, h * 3), (16, 16, 24))
for k, i in enumerate([min(int(len(frames) * j / 9), len(frames) - 1) for j in range(9)]):
    strip.paste(frames[i], ((k % 3) * w, (k // 3) * h))
strip.save("demo/physpoly_filmstrip.png")
frames[len(frames) // 3].save("demo/physpoly_still1.png")
frames[2 * len(frames) // 3].save("demo/physpoly_still2.png")
for name in ("demo/physpoly.gif", "demo/physpoly_filmstrip.png"):
    im = Image.open(name).convert("L")
    hh = im.histogram()
    r = 1 - hh[255] / sum(hh)
    assert r > 0.05, name + " выглядит пустым (non-white %.3f)" % r
    print("ok - %s: non-white %.3f, %d bytes" % (name, r, os.pathsize if False else os.path.getsize(name)))
