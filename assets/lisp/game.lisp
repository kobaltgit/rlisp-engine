;; game.lisp — Уровень 4: игра «Выживание» (демо MVP по ТЗ §12).
;; ВЕСЬ код ниже — Lisp: логика, физика, ECS, рендер, HUD.
;; Правьте этот файл на лету — ядро подхватит изменения без перезапуска.

(window-create *screen-w* *screen-h* "SURVIVAL :: rlisp-engine")

(define *game* (make-table))
(defmacro defgame (name . opts)
  `(begin
     (table-set! *game* 'name ',name)
     (table-set! *game* 'config ',opts)
     ',name))

(defgame survival title "SURVIVAL" genre "arcade" engine "rlisp")

;; ——— persistent-состояние: переживает hot reload (ТЗ §8) ———
(defpersistent *score* 0)
(defpersistent *kills* 0)
(defpersistent *wave* 1)
(defpersistent *player* (make-table))
(defpersistent *bullets* '())
(defpersistent *game-over* #f)
(defpersistent *spawn-timer* 1.0)
(defpersistent *wave-timer* 0.0)
(defpersistent *fire-timer* 0.0)

;; ——— аудиоэффекты 2D-игры ———
(audio-preset-laser 'laser 960.0)
(audio-preset-explosion 'explosion)
(audio-preset-hit 'hit)

;; ——— типы врагов: данные + макрос, новый враг — 3 строки Lisp (критерий G6) ———
(define *enemy-types* (make-table))
(defmacro defenemy (name . kvs)
  `(table-set! *enemy-types* ',name ',kvs))

(defenemy grunt
  speed 42
  w 10 h 10
  hp 1
  color (0.95 0.22 0.25)
  score 10)

(defenemy brute
  speed 26
  w 16 h 16
  hp 3
  color (0.75 0.4 0.95)
  score 30)

(define *player-speed* 115)
(define *fire-cooldown* 0.38)

;; ——— звёздное поле: пересоздаётся при перезагрузке (видно, что мир живой) ———
(define (make-stars n)
  (if (<= n 0) '()
      (cons (list (random *screen-w*) (random *screen-h*)) (make-stars (- n 1)))))
(define *stars* (make-stars 70))

;; игрок инициализируется только при первом старте;
;; при перезагрузке таблица *player* восстанавливается как persistent
(if (table-has? *player* 'x) #f
    (begin (table-set! *player* 'x (/ *screen-w* 2))
           (table-set! *player* 'y (/ *screen-h* 2))))

;; ——— компоненты (ECS DSL, ecs.lisp) ———
(defcomponent position x 0.0 y 0.0 vx 0.0 vy 0.0)
(defcomponent body w 10 h 10)
(defcomponent health hp 1)
(defcomponent kind-tag kind grunt)

;; ——— спавн врага на случайной стороне экрана ———
(define (spawn-enemy kind)
  (let* ((cfg (table-get *enemy-types* kind '()))
         (w (get-kv cfg 'w 10))
         (h (get-kv cfg 'h 10))
         (side (random 4))
         (x 0) (y 0))
    (cond ((= side 0) (begin (set! x (random (- *screen-w* 60))) (set! y -4)))
          ((= side 1) (begin (set! x (random (- *screen-w* 60))) (set! y (- *screen-h* 8))))
          ((= side 2) (begin (set! x -4) (set! y (random (- *screen-h* 60)))))
          (else (begin (set! x (- *screen-w* 8)) (set! y (random (- *screen-h* 60))))))
    (let ((id (make-entity 'position 'body 'health 'kind-tag)))
      (entity-set! id 'kind kind)
      (entity-set! id 'x (+ x 30))
      (entity-set! id 'y (+ y 6))
      (entity-set! id 'w w)
      (entity-set! id 'h h)
      (entity-set! id 'hp (get-kv cfg 'hp 1))
      (entity-set! id 'mul (+ 1.0 (* 0.06 (- *wave* 1))))
      id)))

;; ——— система: преследование игрока (скорость читается из *enemy-types*,
;;      поэтому hot reload меняет поведение уже выпущенных врагов) ———
(defsystem chase-system (dt id e)
  (let* ((px (table-get *player* 'x 0.0))
         (py (table-get *player* 'y 0.0))
         (cfg (table-get *enemy-types* (table-get e 'kind 'grunt) '()))
         (sp (* (get-kv cfg 'speed 40) (table-get e 'mul 1.0)))
         (d (dir-to (table-get e 'x 0.0) (table-get e 'y 0.0) px py)))
    (entity-set! id 'vx (* (car d) sp))
    (entity-set! id 'vy (* (cadr d) sp))))

;; ——— стрельба: автоприцел по ближайшему врагу ———
(define (nearest-enemy)
  (let ((best #f) (best-d 1e18))
    (for-each-entity
     (lambda (id e)
       (let* ((dx (- (table-get e 'x 0.0) (table-get *player* 'x 0.0)))
              (dy (- (table-get e 'y 0.0) (table-get *player* 'y 0.0)))
              (d (+ (* dx dx) (* dy dy))))
         (if (< d best-d)
             (begin (set! best-d d) (set! best e))
             #f))))
    best))

(define (fire!)
  (if (> (length *bullets*) 40) #f
      (let ((target (nearest-enemy)))
        (if target
            (let* ((px (+ (table-get *player* 'x 0.0) 6.0))
                   (py (+ (table-get *player* 'y 0.0) 6.0))
                   (d (dir-to px py (table-get target 'x 0.0) (table-get target 'y 0.0)))
                   (b (make-table)))
              (table-set! b 'x px)
              (table-set! b 'y py)
              (table-set! b 'vx (* (car d) 260.0))
              (table-set! b 'vy (* (cadr d) 260.0))
              (set! *bullets* (cons b *bullets*))
              (play-sfx 'laser 0.5 1.1))
            #f))))

(define (update-fire dt)
  (set! *fire-timer* (- *fire-timer* dt))
  (if (<= *fire-timer* 0.0)
      (begin (set! *fire-timer* *fire-cooldown*) (fire!))
      #f))

;; ——— пули: движение, попадания (AABB из physics.lisp), счёт ———
(define (bullet-alive? b)
  (let ((x (table-get b 'x 0.0)) (y (table-get b 'y 0.0)))
    (and (> x -10.0) (< x (+ *screen-w* 10)) (> y -10.0) (< y (+ *screen-h* 10)))))

(define (enemy-score kind)
  (get-kv (table-get *enemy-types* kind '()) 'score 0))

(define (update-bullets dt)
  (for-each
   (lambda (b)
     (table-set! b 'x (+ (table-get b 'x 0.0) (* (table-get b 'vx 0.0) dt)))
     (table-set! b 'y (+ (table-get b 'y 0.0) (* (table-get b 'vy 0.0) dt))))
   *bullets*)
  (let ((entries (table-entries *entities*)))
    (for-each
     (lambda (b)
       (for-each
        (lambda (entry)
          (let ((id (car entry)) (e (cadr entry)))
            (if (aabb-collide? (table-get b 'x 0.0) (table-get b 'y 0.0) 3 3
                               (table-get e 'x 0.0) (table-get e 'y 0.0)
                               (table-get e 'w 10) (table-get e 'h 10))
                (let ((hp (- (table-get e 'hp 1) 1)))
                  (if (<= hp 0)
                      (begin (entity-remove! id)
                             (set! *score* (+ *score* (enemy-score (table-get e 'kind 'grunt))))
                             (set! *kills* (+ *kills* 1))
                             (play-sfx 'explosion 0.8 1.0)
                             (table-set! b 'dead #t))
                      (begin
                        (entity-set! id 'hp hp)
                        (play-sfx 'hit 0.5 1.2))))
                #f)))
        entries))
     *bullets*)
    (set! *bullets*
          (filter (lambda (b) (and (bullet-alive? b) (not (table-get b 'dead #f)))) *bullets*))
    #t))

;; ——— игрок: WASD ———
(define (update-player dt)
  (let ((dx 0) (dy 0))
    (if (key-down? "a") (set! dx (- dx 1)) #f)
    (if (key-down? "d") (set! dx (+ dx 1)) #f)
    (if (key-down? "w") (set! dy (- dy 1)) #f)
    (if (key-down? "s") (set! dy (+ dy 1)) #f)
    (let* ((diag (and (not (= dx 0)) (not (= dy 0))))
           (k (if diag 0.7071 1.0))
           (nx (clamp (+ (table-get *player* 'x 0.0) (* dx *player-speed* dt k)) 0.0 (- *screen-w* 13)))
           (ny (clamp (+ (table-get *player* 'y 0.0) (* dy *player-speed* dt k)) 0.0 (- *screen-h* 13))))
      (table-set! *player* 'x nx)
      (table-set! *player* 'y ny))))

;; ——— волны ———
(define (update-waves dt)
  (set! *spawn-timer* (- *spawn-timer* dt))
  (if (<= *spawn-timer* 0.0)
      (begin
        (spawn-enemy (if (and (> *wave* 1) (= (random 3) 0)) 'brute 'grunt))
        (set! *spawn-timer* (max 0.5 (- 2.4 (* 0.18 *wave*)))))
      #f)
  (set! *wave-timer* (+ *wave-timer* dt))
  (if (> *wave-timer* 12.0)
      (begin (set! *wave-timer* 0.0)
             (set! *wave* (+ *wave* 1))
             (spawn-enemy 'brute))
      #f))

;; ——— столкновение враг-игрок → game over ———
(define (handle-collisions)
  (if *game-over* #f
      (begin
        (for-each-entity
         (lambda (id e)
           (if (and (not *game-over*)
                    (aabb-collide? (table-get *player* 'x 0.0) (table-get *player* 'y 0.0) 12 12
                                   (table-get e 'x 0.0) (table-get e 'y 0.0)
                                   (table-get e 'w 10) (table-get e 'h 10)))
               (begin
                 (set! *game-over* #t)
                 (play-sfx 'explosion 1.0 0.6))
               #f)))
        #t)))

(define (reset-game)
  (set! *score* 0) (set! *kills* 0) (set! *wave* 1)
  (set! *game-over* #f)
  (set! *bullets* '())
  (table-clear! *entities*)
  (table-set! *player* 'x (/ *screen-w* 2))
  (table-set! *player* 'y (/ *screen-h* 2))
  (set! *spawn-timer* 1.0) (set! *wave-timer* 0.0) (set! *fire-timer* 0.0)
  (play-sfx 'laser 0.7 1.4)
  (print "reset-game: новый забег"))

(define (check-restart)
  (if (and *game-over* (key-pressed? "r")) (reset-game) #f))

;; ——— главный update: его вызывает Rust-ядро каждый кадр ———
(define (update dt)
  (poll-input)
  (if *game-over*
      (check-restart)
      (begin
        (update-player dt)
        (run-systems dt)                                ;; chase-system: скорости
        (integrate-batch! (table-keys *entities*) dt)   ;; SIMD: все враги одним вызовом
        (update-fire dt)
        (update-bullets dt)
        (update-waves dt)
        (handle-collisions)
        #t)))

;; ——— отрисовка: тоже Lisp (graphics.lisp) ———
(define (draw-enemy e)
  (let* ((cfg (table-get *enemy-types* (table-get e 'kind 'grunt) '()))
         (col (get-kv cfg 'color (list 1.0 0.2 0.2)))
         (x (table-get e 'x 0.0)) (y (table-get e 'y 0.0))
         (w (table-get e 'w 10)) (h (table-get e 'h 10)))
    (gpu-fill-rect x y w h (car col) (cadr col) (caddr col))
    (gpu-fill-rect (+ x 2) (+ y 2) (- w 4) (- h 4)
                   (* (car col) 0.45) (* (cadr col) 0.45) (* (caddr col) 0.45))))

(define (draw-bullet b)
  (gpu-fill-rect (table-get b 'x 0.0) (table-get b 'y 0.0) 3 3 1.0 0.85 0.3))

(define (draw-player)
  (let ((x (table-get *player* 'x 0.0)) (y (table-get *player* 'y 0.0)))
    (gpu-fill-rect x y 12 12 0.15 0.85 1.0)
    (gpu-fill-rect (+ x 3) (+ y 3) 6 6 0.9 1.0 1.0)
    (gpu-fill-rect (+ x 4) (- y 3) 4 4 0.15 0.85 1.0)))

(define (draw-hud)
  (draw-text (string-append "SCORE " (number->string *score*)) 6 5 1 (list 1.0 1.0 1.0))
  (draw-text (string-append "WAVE " (number->string *wave*)) 6 13 1 (list 0.65 0.72 0.9))
  (draw-text (string-append "KILLS " (number->string *kills*)) 80 13 1 (list 0.5 0.56 0.7)))

(define (draw-game-over)
  (draw-text "GAME OVER"
             (/ (- *screen-w* (text-width "GAME OVER" 3)) 2) 92 3 (list 1.0 0.3 0.3))
  (draw-text "PRESS R TO RESTART"
             (/ (- *screen-w* (text-width "PRESS R TO RESTART" 1)) 2) 132 1 (list 1.0 1.0 1.0))
  (draw-text (string-append "SCORE " (number->string *score*))
             (/ (- *screen-w* (text-width "SCORE 0000" 1)) 2) 146 1 (list 1.0 0.9 0.5)))

(define (draw)
  (with-frame
   (for-each (lambda (s) (gpu-fill-rect (car s) (cadr s) 2 2 0.35 0.38 0.55)) *stars*)
   (for-each-entity (lambda (id e) (draw-enemy e)))
   (for-each (lambda (b) (draw-bullet b)) *bullets*)
   (draw-player)
   (draw-hud)
   (if *game-over* (draw-game-over) #f)))
