;; replay_demo.lisp — детерминизм как проверяемое свойство.
;;
;; Сцена минимальная НАМЕРЕННО: она не рисует красиво, она доказывает, что
;; движок воспроизводим кадр в кадр. Всё состояние — три числа, источник
;; случайности — только (random) с явным seed, ввод — только клавиши.
;;
;;   # записать прогон
;;   cargo run --release -- --script assets/lisp/replay_demo.lisp \
;;       --fast --frames 240 --record saves/demo.rlrp
;;
;;   # воспроизвести и сверить hash-цепочку кадров
;;   cargo run --release -- --script assets/lisp/replay_demo.lisp \
;;       --fast --frames 240 --replay saves/demo.rlrp
;;
;; Ожидаемый вывод: те же 240 кадров, тот же chain, вердикт OK.
;; Сцена печатает свою цепочку состояния независимо от движка:
;; (hash-int (list x y n) 1e9) — если она совпала, совпала и логика, а не
;; только пиксели.

(load "core.lisp")
(load "input.lisp")

(define *seed* 20261014)
(random-seed *seed*)

(define *x* 160.0)
(define *y* 120.0)
(define *n* 0)
(define *sum* 0)
(define *state-chain* 0)

;; маленький FNV-подобный шаг в Лиспе: 32-битная арифметика через mod
(define (mix h v)
  (mod (+ (* h 16777619) (+ v 2166136261)) 4294967296))

(define (update dt)
  (poll-input)
  ;; ввод двигает тело — это и есть записываемый/воспроизводимый поток
  (cond ((key-down? "w") (set! *y* (- *y* (* 40.0 dt))))
        ((key-down? "s") (set! *y* (+ *y* (* 40.0 dt))))
        ((key-down? "a") (set! *x* (- *x* (* 40.0 dt))))
        ((key-down? "d") (set! *x* (+ *x* (* 40.0 dt)))))
  ;; случайность — только из seed-ГПСЧ
  (let* ((jitter (random-int -3 3)))
    (set! *n* (+ *n* 1))
    (set! *sum* (+ *sum* jitter))
    (set! *state-chain* (mix *state-chain* (abs jitter))))
  (set! *x* (max 2.0 (min 310.0 *x*)))
  (set! *y* (max 2.0 (min 232.0 *y*))))

(define (draw)
  (gpu-clear 0.06 0.07 0.09)
  ;; рамка поля
  (gpu-fill-rect 0 0 320 2 0.25 0.30 0.38)
  (gpu-fill-rect 0 238 320 2 0.25 0.30 0.38)
  ;; тело: цвет зависит от накопленной суммы — логика видна в пикселях
  (let* ((t (mod *sum* 100)))
    (gpu-fill-rect (floor *x*) (floor *y*) 6 6
                   (+ 0.3 (* 0.007 t)) 0.55 (- 0.9 (* 0.006 t))))

  ;; маркер фазы: каждые 60 кадров — белая полоса слева
  (if (= (mod *n* 60) 0)
      (gpu-fill-rect 2 2 3 30 1.0 1.0 1.0))
  (gpu-present))

(define (frame-end)
  #t)
