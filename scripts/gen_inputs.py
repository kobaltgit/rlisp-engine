#!/usr/bin/env python3
"""Генерирует сценарий ввода для демо-захвата: игрок кружит по эллипсу вокруг центра.

Формат: «ВРЕМЯ КЛАВИША down|up». Движок «залипает» клавишу на 0.2с (input.lisp),
поэтому нажатия надо повторять каждые ~0.12с.
"""
import math

T_END = 18.0
STEP = 0.05
CX, CY = 160, 120
RX, RY = 95, 70
W = 0.55          # рад/с
RESEND = 0.12

def target(t):
    a = W * t
    return CX + math.cos(a) * RX, CY + math.sin(a) * RY

def main():
    lines = []
    prev = set()
    last_sent = {}
    t = 0.0
    while t <= T_END:
        tx, ty = target(t)
        cur = set()
        if tx < CX - 4:
            cur.add("a")
        elif tx > CX + 4:
            cur.add("d")
        if ty < CY - 4:
            cur.add("w")
        elif ty > CY + 4:
            cur.add("s")
        for k in sorted(prev - cur):
            lines.append(f"{t:.2f} {k} up")
        for k in sorted(cur):
            if t - last_sent.get(k, -1e9) >= RESEND:
                lines.append(f"{t:.2f} {k} down")
                last_sent[k] = t
        prev = cur
        t += STEP
    print("\n".join(lines))

if __name__ == "__main__":
    main()
