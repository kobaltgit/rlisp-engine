;; main_valley.lisp — точка входа «ДОЛИНА» (этап 27).
;; Запуск: rlisp-engine --script assets/lisp/main_valley.lisp
;; Большой открытый ландшафт: fbm-рельеф с озером и горным ободом,
;; день-ночь, лес с ветром, трава, вода. Минимум системных DSL (вершинные
;; цвета, без текстурных файлов) — работает и в wasm-песочнице.

(load "core.lisp")
(load "textures.lisp")  ;; tex-canvas/texture-upload для облаков
(load "font.lisp")      ;; данные: растровый шрифт
(load "graphics.lisp")  ;; системный DSL: кадр, текст, примитивы 2D (HUD)
(load "input.lisp")     ;; системный DSL: состояние клавиш
(load "render3d.lisp")  ;; системный DSL: матрицы, камера, меши, draw (+батч)
(load "valley.lisp")    ;; сцена «ДОЛИНА» (этап 27)

(print "rlisp-engine DOLINA: большой открытый ландшафт; update/draw на Lisp")
