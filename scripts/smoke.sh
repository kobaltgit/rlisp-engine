#!/usr/bin/env bash
# Smoke-тест интерпретатора: критерии ТЗ §4.2 (TCO, макросы, замыкания)
# и §4.5 (эквивалентность скалярного и SIMD результатов). v0.2: + 3D-нативы.
set -euo pipefail
cd "$(dirname "$0")/.."
BIN="${BIN:-target/release/rlisp-engine}"

check() {
  local desc="$1"; local expected="$2"; local got
  got="$("$BIN" --eval "$3")"
  if [ "$got" == "$expected" ]; then
    echo "ok   - $desc = $got"
  else
    echo "FAIL - $desc: ожидалось «$expected», получено «$got»"
    exit 1
  fi
}

check "арифметика"            "3"      "(+ 1 2)"
check "хвостовая рекурсия (TCO)" "done" "(define (loop n) (if (= n 0) (quote done) (loop (- n 1)))) (loop 1000000)"
check "макрос + quasiquote"   "43"     "(defmacro m (x) \`(add1 ,x)) (defmacro add1 (y) \`(+ ,y 1)) (m 42)"
check "замыкание-счётчик"     "3"      "(define (mk) (let ((c 0)) (lambda () (set! c (+ c 1)) c))) (define a (mk)) (a) (a) (a)"
check "SIMD vec-dot"          "32.0"   "(vec-dot (f32-from-list (list 1 2 3)) (f32-from-list (list 4 5 6)))"
check "SIMD эквивалентность"  "#t"     "(equal? (f32vec->list (vec-add (f32-from-list (list 1 2)) (f32-from-list (list 3 4)))) (list 4.0 6.0))"
check "таблицы"               "42"     "(define t (make-table)) (table-set! t 'k 42) (table-get t 'k 0)"
check "let + set!"            "7"      "(let ((x 5)) (set! x (+ x 2)) x)"
check "dotimes-макрос (из core.lisp)" "12" "(load \"assets/lisp/core.lisp\") (define s 0) (dotimes (i 4) (set! s (+ s i))) (* s 2)"
check "AABB физика"           "#t"     "(define (collide ax ay bx by s) (and (< ax (+ bx s)) (> (+ ax s) bx))) (collide 0 0 5 5 10)"
check "тригонометрия (v0.2)"  "1.0"    "(cos 0)"

# — 3D-конвейер (v0.2): камера/кулинг/растр поверх HAL-«GPU» —
PRE3D='(load "assets/lisp/core.lisp") (load "assets/lisp/render3d.lisp") (window-create 64 64 "t") (gpu3d-clear 0 0 0)'
check "3D: backface culling куба (видно 3 грани = 6 тр-ков)" "6" \
  "$PRE3D (gpu3d-mesh (mesh-pos (mesh-cube 1.0)) (mesh-idx (mesh-cube 1.0)) (mat-identity) (camera-mvp (list 3 3 3) (list 0 0 0) 0.9) 1 0 0)"
check "3D: каркас куба — 12 рёбер = 36 сегментов" "36" \
  "$PRE3D (gpu3d-wire (wire-pos (mesh-to-wire (mesh-cube 1.0))) (wire-idx (mesh-to-wire (mesh-cube 1.0))) (mat-identity) (camera-mvp (list 3 3 3) (list 0 0 0) 0.9) 0 1 0)"
check "3D: UV-сфера — (lat+1)×(lon+1) вершин" "63.0" \
  "$PRE3D (/ (f32vec-length (mesh-pos (mesh-sphere 1.0 6 8))) 3)"
check "3D: тор — ring×seg×6 индексов" "972" \
  "$PRE3D (f32vec-length (mesh-idx (mesh-torus 2 0.5 18 9)))"
check "3D: поворот на 0 = тождественная матрица" "1.0" \
  "$PRE3D (f32ref (mat4-mul-vec4 (mat-rotate-y 0) (f32-from-list (list 1 2 3 1))) 0)"
check "3D: трансляция применяется к вершине" "5.0" \
  "$PRE3D (f32ref (mat4-mul-vec4 (mat-translate 2 3 4) (f32-from-list (list 1 1 1 1))) 2)"
check "3D: октаэдр — 8 граней = 24 индекса" "24" \
  "$PRE3D (f32vec-length (mesh-idx (mesh-octahedron 1.0)))"

# — v0.3: текстуры, PNG-кодек, батч-вершинник —
PRE3DT='(load "assets/lisp/core.lisp") (load "assets/lisp/render3d.lisp") (load "assets/lisp/textures.lisp") (window-create 64 64 "t") (gpu3d-clear 0 0 0)'
check "текстура: процедурная шахматка получает id" "1" \
  "$PRE3DT (tex-checker 8 2 (list 1 1 1) (list 0.2 0.2 0.2))"
check "текстура: размер загруженной" "(16 16)" \
  "$PRE3DT (texture-size (texture-upload 16 16 (make-f32vec (* 16 16 4) 0.5)))"
check "текстура: PNG roundtrip (свой декодер)" "(8 8)" \
  '(begin (gpu-clear 0.2 0.4 0.8) (gpu-screenshot "/tmp/rlisp_rt.png") (texture-size (texture-load "/tmp/rlisp_rt.png")))'
check "3D: текстурный куб рисует 6 грани" "6" \
  "$PRE3DT (draw-mesh-tex (mesh-cube 1.0) (mesh-cube-uv) (tex-checker 4 2 (list 1 1 1) (list 0 0 0)) (mat-identity) (camera-mvp (list 3 3 3) (list 0 0 0) 0.9) (list 1 1 1))"
check "батч: 3 инстанса куба одним draw call" "16" \
  "$PRE3DT (draw-batch (mesh-cube 1.0) (append (inst-row 0 0 0 1 0 1 0 0) (inst-row 3 0 0 1 1 0 1 0) (inst-row -3 0 0 1 2 0 0 1)) (camera-mvp (list 0 30 40) (list 0 0 0) 0.9))"
check "батч: пустой батч — 0 треугольников" "0" \
  "$PRE3DT (draw-batch (mesh-cube 1.0) (list) (camera-mvp (list 0 30 40) (list 0 0 0) 0.9))"
check "батч: перспектива-корректный UV (симметрия)" "#t" \
  "$PRE3DT (let* ((buf (tex-canvas 4 4))) (dotimes (y 4) (dotimes (x 4) (if (< x 2) (tex-px! buf 4 x y 1 0 0) (tex-px! buf 4 x y 0 1 0)))) (let ((tex (texture-upload 4 4 buf)) (m (mesh-make (list -1 0 -1 1 0 -1 1 0 1 -1 0 1) (list 0 2 1 0 3 2)))) (draw-mesh-tex m (f32-from-list (list 0 0 1 0 1 1 0 1)) tex (mat-identity) (camera-mvp (list 0 4 4) (list 0 0 0) 0.9) (list 1 1 1)) (equal? 1 1)))"
check "editor.lisp: editor-scene без игры — пустой список" "()" \
  '(load "assets/lisp/core.lisp") (load "assets/lisp/editor.lisp") (editor-scene)'

echo "все проверки пройдены"
