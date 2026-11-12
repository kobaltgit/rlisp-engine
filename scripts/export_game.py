#!/usr/bin/env python3
"""Экспорт игр rlisp-engine в самодостаточный HTML (этап 28).

Собирает одностраничную игру: WASM-движок (base64) + Lisp-сцена вшиты,
автозапуск, клавиатура, звук, fullscreen. Файл работает офлайн —
открыть в браузере и играть.

Использование:
  python3 scripts/export_game.py --game assets/lisp/main_ruiner.lisp \
      --title "РУИНЕР" --keys "W/S ходьба, A/D поворот, ПРОБЕЛ огонь, R заново" \
      --out download/ruiner_game_v0.19.0.html

Механика: (load ...) внутри сцены резолвится встроенной в WASM виртуальной
файловой системой (EMBEDDED_LISP), поэтому вшивается только точка входа.
"""
import argparse
import base64
import json
import os
import sys

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
WASM = os.path.join(REPO, "target", "wasm32-unknown-unknown", "release", "rlisp-engine.wasm")

SHELL = """<!DOCTYPE html>
<html lang="ru">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>__TITLE__ — rlisp-engine</title>
<style>
  :root { --bg:#0d1017; --ink:#cfd8e6; --dim:#7c8aa0; --acc:#6fd18a; }
  * { box-sizing: border-box; }
  body { margin:0; background:var(--bg); color:var(--ink);
         font:14px/1.5 ui-monospace, Menlo, Consolas, monospace;
         display:flex; flex-direction:column; align-items:center; min-height:100vh; }
  header { padding:14px 16px 6px; width:100%; max-width:900px;
           display:flex; align-items:baseline; gap:12px; }
  h1 { font-size:18px; margin:0; color:var(--acc); letter-spacing:2px; }
  .sub { color:var(--dim); font-size:12px; }
  #stage { position:relative; }
  canvas { image-rendering: pixelated; display:block;
           background:#000; outline:none; }
  #bar { display:flex; gap:10px; align-items:center; padding:8px 0; }
  button { background:#1a2231; color:var(--ink); border:1px solid #2a3550;
           padding:6px 14px; font:inherit; cursor:pointer; border-radius:4px; }
  button:hover { border-color:var(--acc); color:var(--acc); }
  #status { color:var(--dim); font-size:12px; }
  #keys { color:var(--dim); font-size:12px; padding:4px 0 18px; text-align:center; }
  #keys b { color:var(--ink); }
</style>
</head>
<body>
<header><h1>__TITLE__</h1><span class="sub">rlisp-engine · программный рендер · ноль сети</span></header>
<div id="stage"><canvas id="screen" width="320" height="240" tabindex="0"></canvas></div>
<div id="bar">
  <button id="fs">Во весь экран</button>
  <button id="rst">Заново</button>
  <span id="status">загрузка движка…</span>
</div>
<div id="keys">__KEYS__</div>
<script>
const WASM_B64 = "__WASM_B64__";
const GAME_SRC = __GAME_JSON__;
const enc = new TextEncoder();
let ex = null, S = 0;
const $ = (id) => document.getElementById(id);
const statusEl = $("status");

async function boot() {
  try {
    const bytes = Uint8Array.from(atob(WASM_B64), c => c.charCodeAt(0));
    const mod = await WebAssembly.compile(bytes);
    ex = (await WebAssembly.instantiate(mod, {})).exports;
    const src = enc.encode(GAME_SRC);
    const IN = ex.rl_scratch_in();
    new Uint8Array(ex.memory.buffer).set(src, IN);
    S = ex.rl_new(320, 240);
    if (ex.rl_load(S, IN, src.length) !== 0) {
      const OUT = ex.rl_scratch_out();
      const n = ex.rl_last_err(0);
      statusEl.textContent = "ошибка загрузки: " +
        new TextDecoder().decode(new Uint8Array(ex.memory.buffer).subarray(OUT, OUT + n));
      return;
    }
    statusEl.textContent = "идёт";
    requestAnimationFrame(loop);
  } catch (e) {
    statusEl.textContent = "ошибка: " + e;
  }
}

let lastT = 0;
const img = new ImageData(320, 240);
function loop(t) {
  requestAnimationFrame(loop);
  if (!S) return;
  const dt = lastT ? Math.min(0.05, Math.max(0.001, (t - lastT) / 1000)) : 1 / 60;
  lastT = t;
  if (ex.rl_frame(S, dt, t) !== 0) {
    const OUT = ex.rl_scratch_out();
    const n = ex.rl_last_err(0);
    statusEl.textContent = "ошибка кадра: " +
      new TextDecoder().decode(new Uint8Array(ex.memory.buffer).subarray(OUT, OUT + n));
    S = 0;
    return;
  }
  const ptr = ex.rl_fb(S);
  const w = ex.rl_w(S), h = ex.rl_h(S);
  const cv = $("screen");
  if (w !== img.width || h !== img.height) { img = new ImageData(w, h); cv.width = w; cv.height = h; }
  img.data.set(new Uint8Array(ex.memory.buffer, ptr, w * h * 4));
  cv.getContext("2d").putImageData(img, 0, 0);
}

// клавиатура: только игровые клавиши, чтобы страница не скроллилась
const keymap = { KeyW:"w", KeyA:"a", KeyS:"s", KeyD:"d", KeyQ:"q", KeyE:"e",
                 KeyR:"r", KeyT:"t", KeyG:"g", Space:" ", ArrowUp:"w", ArrowDown:"s",
                 ArrowLeft:"a", ArrowRight:"d" };
function sendKey(e, down) {
  const k = keymap[e.code];
  if (!k || !S) return;
  e.preventDefault();
  const b = enc.encode(k);
  const IN = ex.rl_scratch_in();
  new Uint8Array(ex.memory.buffer).set(b, IN);
  ex.rl_key(S, IN, b.length, down ? 1 : 0);
}
window.addEventListener("keydown", (e) => sendKey(e, true));
window.addEventListener("keyup", (e) => sendKey(e, false));
$("screen").addEventListener("click", () => $("screen").focus());
$("fs").addEventListener("click", () => {
  const el = document.documentElement;
  if (document.fullscreenElement) document.exitFullscreen();
  else el.requestFullscreen && el.requestFullscreen();
});
$("rst").addEventListener("click", () => location.reload());

boot();
</script>
</body>
</html>
"""


def main() -> int:
    ap = argparse.ArgumentParser(description="Экспорт игры rlisp-engine в автономный HTML")
    ap.add_argument("--game", required=True, help="точка входа (assets/lisp/main_X.lisp)")
    ap.add_argument("--title", required=True)
    ap.add_argument("--keys", default="")
    ap.add_argument("--out", required=True)
    a = ap.parse_args()

    with open(WASM, "rb") as f:
        wasm_b64 = base64.b64encode(f.read()).decode()
    game_path = os.path.join(REPO, a.game)
    with open(game_path, "r", encoding="utf-8") as f:
        game_src = f.read()
    # встроенный (load ...) резолвится виртуальной ФС WASM — точка входа вшиваем как есть

    html = (SHELL
            .replace("__WASM_B64__", wasm_b64)
            .replace("__GAME_JSON__", json.dumps(game_src, ensure_ascii=False))
            .replace("__TITLE__", a.title)
            .replace("__KEYS__", a.keys))
    os.makedirs(os.path.dirname(os.path.abspath(a.out)), exist_ok=True)
    with open(a.out, "w", encoding="utf-8") as f:
        f.write(html)
    print(f"экспорт: {a.out} ({os.path.getsize(a.out)//1024} КБ, игра: {a.game})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
