;; graphics.lisp — Уровень 3: Graphics DSL поверх сырых gpu-* команд HAL.
;; HAL умеет только заливать прямоугольники и показывать кадр.
;; Камера, спрайты, текст, кадр целиком — это Lisp, и это можно заменить
;; правкой этого файла без перекомпиляции Rust (критерий G6 из ТЗ).

(define *screen-w* 320)
(define *screen-h* 240)

;; весь кадр: очистка → тело → present
(defmacro with-frame body
  `(begin (gpu-clear 0.03 0.03 0.07) ,@body (gpu-present)))

(define (draw-rect x y w h color)
  (gpu-fill-rect x y w h (car color) (cadr color) (caddr color)))

;; — текст: глиф 3x5 → fill-rect на каждый включённый пиксель —
(define (draw-glyph-row bits c x y scale col)
  (if (>= c 3) #f
      (begin
        (if (bit? bits (- 2 c))
            (gpu-fill-rect (+ x (* c scale)) y scale scale
                           (car col) (cadr col) (caddr col))
            #f)
        (draw-glyph-row bits (+ c 1) x y scale col))))

(define (draw-glyph-rows rows r x y scale col)
  (if (>= r 5) #f
      (begin
        (draw-glyph-row (nth rows r) 0 x (+ y (* r scale)) scale col)
        (draw-glyph-rows rows (+ r 1) x y scale col))))

(define (draw-glyph ch x y scale col)
  (let ((rows (table-get *font* ch #f)))
    (if rows (draw-glyph-rows rows 0 x y scale col) #f)))

(define (draw-text-rec str i x y scale col)
  (if (>= i (string-length str)) #f
      (begin
        (draw-glyph (string-at str i) x y scale col)
        (draw-text-rec str (+ i 1) (+ x (* 4 scale)) y scale col))))

(define (draw-text str x y scale col)
  (draw-text-rec str 0 x y scale col))

(define (text-width str scale)
  (* 4 scale (string-length str)))
