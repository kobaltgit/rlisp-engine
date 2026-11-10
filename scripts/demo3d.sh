#!/usr/bin/env bash
# ДЕМО HOT RELOAD В 3D (v0.2): движок «ORBITAL» работает, мы правим game3d.lisp
# на лету, ядро пересобирает Lisp-окружение и переносит persistent-состояние.
# Эффект в кадрах: на t=9с враги grunt становятся ОРАНЖЕВЫМИ и почти вдвое быстрее.
set -euo pipefail
cd "$(dirname "$0")/.."
BIN="${BIN:-target/release/rlisp-engine}"

mkdir -p demo/frames3d
cp assets/lisp/game3d.lisp /tmp/game3d.lisp.bak
restore() { cp /tmp/game3d.lisp.bak assets/lisp/game3d.lisp; echo "[demo] game3d.lisp восстановлен"; }
trap restore EXIT

python3 scripts/gen_inputs.py > demo/inputs3d.txt

(
  sleep 9
  sed -i 's/\*grunt-speed\* 16/\*grunt-speed\* 30/; s/(0.95 0.25 0.25)/(1.0 0.55 0.12)/' assets/lisp/game3d.lisp
  echo "[demo] t=9с: game3d.lisp изменён на диске → grunt: speed 16→30, цвет красный→оранжевый"
) &

"$BIN" --script assets/lisp/main3d.lisp \
       --capture demo/frames3d --every 6 \
       --frames 540 --fps 30 \
       --input demo/inputs3d.txt 2>&1 | tee demo/run3d.log

python3 scripts/make_gif.py "demo/frames3d/frame_*.png" demo/orbital.gif demo/filmstrip3d.png
rm -rf demo/frames3d
echo "[demo] готово: demo/orbital.gif + demo/filmstrip3d.png + demo/run3d.log"
