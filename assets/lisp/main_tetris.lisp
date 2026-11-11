;; main_tetris.lisp — Точка входа для «НЕОНОВОГО 3D-ТЕТРИСА»
;; Запуск на GPU:
;;   cargo run --release --features wgpu -j 1 -- --wgpu --serve --fps 60 --width 640 --height 480 --script assets/lisp/main_tetris.lisp
;; В терминале:
;;   cargo run --release -j 1 -- --ascii --stdin-keys --fps 60 --script assets/lisp/main_tetris.lisp

(load "core.lisp")       ;; базовая библиотека + defpersistent
(load "font.lisp")       ;; растровый шрифт
(load "graphics.lisp")   ;; системный 2D DSL: текст, примитивы (HUD)
(load "input.lisp")      ;; обработка клавиатуры
(load "render3d.lisp")   ;; 3D рендер, матрицы, тени, точечный свет
(load "textures.lisp")   ;; процедурные текстуры и UV
(load "materials.lisp")  ;; материалы, блики, альфа
(load "editor.lisp")     ;; веб-редактор и MCP мост
(load "audio.lisp")      ;; процедурный чиптюн-синтезатор
(load "tetris.lisp")     ;; логика и 3D-сцена Неонового Тетриса

(print "rlisp-engine NEON TETRIS 3D: WebGPU + динамические тени + Point Light; update/draw на Lisp")
