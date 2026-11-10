;; main.lisp — точка входа: Rust-ядро грузит ЭТОТ файл и дальше живёт в Lisp-мире.
;; Порядок = порядок бутстрапа из ТЗ §9: базовый Lisp → DSL → игра.

(load "core.lisp")       ;; базовая библиотека + defpersistent
(load "font.lisp")       ;; данные: растровый шрифт
(load "graphics.lisp")   ;; системный DSL: кадр, текст, примитивы
(load "input.lisp")      ;; системный DSL: состояние клавиш
(load "ecs.lisp")        ;; системный DSL: сущности и системы
(load "physics.lisp")    ;; системный DSL: AABB + SIMD-интегрирование
(load "textures.lisp")   ;; системный DSL v0.3: текстуры, спрайты
(load "editor.lisp")     ;; системный DSL v0.3: мост редактора и MCP
(load "audio.lisp")      ;; системный DSL v0.3: аудиосистема, SFX пресеты, микшер
(load "game.lisp")       ;; уровень 4: сама игра

(print "rlisp-engine: HAL → интерпретатор → системные DSL → игра; всё готово, update/draw на Lisp")
