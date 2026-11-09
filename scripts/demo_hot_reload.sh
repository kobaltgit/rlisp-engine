#!/usr/bin/env bash
# ДЕМО HOT RELOAD (ТЗ §8): движок работает, мы правим game.lisp на лету,
# ядро пересобирает Lisp-окружение и переносит persistent-состояние.
# Эффект в кадрах: на t=9с враги grunt становятся ОРАНЖЕВЫМИ и в 2 раза быстрее.
set -euo pipefail
cd "$(dirname "$0")/.."
BIN="${BIN:-target/release/rlisp-engine}"

mkdir -p demo/frames
cp assets/lisp/game.lisp /tmp/game.lisp.bak
restore() { cp /tmp/game.lisp.bak assets/lisp/game.lisp; echo "[demo] game.lisp восстановлен"; }
trap restore EXIT

python3 scripts/gen_inputs.py > demo/inputs.txt

(
  sleep 9
  sed -i 's/speed 42/speed 85/; s/color (0.95 0.22 0.25)/color (1.0 0.55 0.12)/' assets/lisp/game.lisp
  echo "[demo] t=9с: game.lisp изменён на диске → grunt: speed 42→85, цвет красный→оранжевый"
) &

"$BIN" --script assets/lisp/main.lisp \
       --capture demo/frames --every 6 \
       --frames 540 --fps 30 \
       --input demo/inputs.txt 2>&1 | tee demo/run.log

echo "[demo] готово: demo/frames/frame_*.png + demo/run.log"
