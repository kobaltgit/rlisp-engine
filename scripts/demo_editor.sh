#!/usr/bin/env bash
# ДЕМО РЕДАКТОРА (v0.3): движок с --serve, редактор открывается в браузере.
# Сценарий для скринкаста:
#   1) http://127.0.0.1:7300 — иерархия, живой вид, REPL;
#   2) клик по врагу в иерархии → инспектор → правь x/hp → «Применить»;
#   3) вкладка «Код» → game3d.lisp → правь цвет врага → Ctrl+S →
#      движок сам делает hot reload, мир меняется без перезапуска;
#   4) MCP для ИИ-агентов: rlisp-engine --mcp --script assets/lisp/main3d.lisp
set -euo pipefail
cd "$(dirname "$0")/.."
BIN="${BIN:-target/release/rlisp-engine}"
PORT="${PORT:-7300}"

echo "редактор: http://127.0.0.1:${PORT}  (Ctrl+C — остановить)"
exec "$BIN" --serve --port "$PORT" --script assets/lisp/main3d.lisp
