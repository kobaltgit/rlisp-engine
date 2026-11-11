;; tetris.lisp — «НЕОНОВЫЙ 3D-ТЕТРИС» (rlisp-engine)
;; Аппаратный 3D WebGPU, сочные неоновые блоки, частицы сгорания линий,
;; управление WASD + стрелки, мгновенный Hard Drop (пробел) и Restart (R).

;; ——— Аудиоэффекты (чиптюн-синтезатор rlisp-engine) ———
(defsound-synth 'tet-move 'triangle 340.0 0.03 0.005 0.01 0.2 0.01 0.0 0.5)
(defsound-synth 'tet-rot  'square   620.0 0.05 0.005 0.02 0.4 0.02 200.0 0.5)
(defsound-synth 'tet-drop 'triangle 130.0 0.08 0.005 0.03 0.3 0.04 -90.0 0.5)
(defsound-synth 'tet-line 'sine     880.0 0.25 0.005 0.05 0.5 0.15 400.0 0.5)
(defsound-synth 'tet-four 'sawtooth 520.0 0.45 0.01 0.1 0.7 0.3 650.0 0.5)
(defsound-synth 'tet-over 'sawtooth 280.0 0.70 0.01 0.2 0.4 0.4 -200.0 0.5)

;; ——— Состояние игры (defpersistent переживает hot-reload) ———
(defpersistent *grid* (make-table))       ;; ключ: (+ (* y 10) x) → (r g b)
(defpersistent *score* 0)
(defpersistent *lines* 0)
(defpersistent *level* 1)
(defpersistent *game-over* #f)

(defpersistent *piece-type* 'T)
(defpersistent *piece-rot* 0)
(defpersistent *piece-x* 3)
(defpersistent *piece-y* 18)
(defpersistent *next-type* 'I)

(defpersistent *fall-timer* 0.0)

;; ——— Система частиц при взрыве сгорающих линий ———
(define *parts* (part-new))
(define *parts-out* (make-f32vec 4096 0.0))
(define *part-mesh* (mesh-cube 0.22))

;; ——— Геометрия и текстуры 3D ———
(define *mesh-block* (mesh-cube 0.47))     ;; кубик 0.94 со скошенной фаской
(define *mesh-wall*  (mesh-cube 0.50))     ;; стойки колодца
(define *mesh-bg*    (mesh-plane 22 10))   ;; задняя стенка
(define *bg-wire*    (list (mesh-pos *mesh-bg*) (mesh-plane-lines 10)))
(define *uv-cube*    (mesh-cube-uv))
(define *tex-panel*  (tex-panel 64))

;; ——— Яркие неоновые цвета тетромино ———
(define (piece-color type)
  (cond
    ((eq? type 'I) (list 0.10 1.00 1.00))   ;; Электрический Циан
    ((eq? type 'O) (list 1.00 0.95 0.15))   ;; Солнечный Неон
    ((eq? type 'T) (list 0.90 0.25 1.00))   ;; Неоновый Маджента/Пурпур
    ((eq? type 'S) (list 0.25 1.00 0.40))   ;; Изумрудный Неон
    ((eq? type 'Z) (list 1.00 0.25 0.30))   ;; Рубиновый Неон
    ((eq? type 'J) (list 0.25 0.50 1.00))   ;; Сапфировый Неон
    ((eq? type 'L) (list 1.00 0.65 0.15))   ;; Янтарный Неон
    (else (list 0.9 0.9 0.9))))

;; ——— Формы 7 классических тетромино в 4 поворотах ———
(define (piece-blocks type rot)
  (let ((r (mod rot 4)))
    (cond
      ((eq? type 'I)
       (cond
         ((= r 0) '((0 1) (1 1) (2 1) (3 1)))
         ((= r 1) '((2 0) (2 1) (2 2) (2 3)))
         ((= r 2) '((0 2) (1 2) (2 2) (3 2)))
         (else    '((1 0) (1 1) (1 2) (1 3)))))
      ((eq? type 'O)
       '((0 0) (1 0) (0 1) (1 1)))
      ((eq? type 'T)
       (cond
         ((= r 0) '((0 0) (1 0) (2 0) (1 1)))
         ((= r 1) '((1 0) (1 1) (2 1) (1 2)))
         ((= r 2) '((1 0) (0 1) (1 1) (2 1)))
         (else    '((1 0) (0 1) (1 1) (1 2)))))
      ((eq? type 'S)
       (cond
         ((or (= r 0) (= r 2)) '((0 0) (1 0) (1 1) (2 1)))
         (else                 '((2 0) (1 1) (2 1) (1 2)))))
      ((eq? type 'Z)
       (cond
         ((or (= r 0) (= r 2)) '((1 0) (2 0) (0 1) (1 1)))
         (else                 '((1 0) (1 1) (2 1) (2 2)))))
      ((eq? type 'J)
       (cond
         ((= r 0) '((0 0) (0 1) (1 1) (2 1)))
         ((= r 1) '((1 2) (1 1) (1 0) (2 0)))
         ((= r 2) '((0 1) (1 1) (2 1) (2 2)))
         (else    '((1 0) (1 1) (1 2) (0 2)))))
      ((eq? type 'L)
       (cond
         ((= r 0) '((2 0) (0 1) (1 1) (2 1)))
         ((= r 1) '((1 0) (1 1) (1 2) (2 2)))
         ((= r 2) '((0 2) (0 1) (1 1) (2 1)))
         (else    '((0 0) (1 0) (1 1) (1 2)))))
      (else '((0 0))))))

(define *all-types* '(I O T S Z J L))

(define (random-piece)
  (let ((idx (random 7)))
    (define (pick lst i)
      (if (= i idx) (car lst)
          (pick (cdr lst) (+ i 1))))
    (pick *all-types* 0)))

;; ——— Проверка коллизий сетки ———
(define (grid-cell-occupied? gx gy)
  (if (or (< gx 0) (>= gx 10) (< gy 0))
      #t
      (if (>= gy 20)
          #f
          (table-has? *grid* (+ (* gy 10) gx)))))

(define (valid-placement? type rot px py)
  (let ((blocks (piece-blocks type rot)))
    (define (check-blocks b)
      (if (null? b)
          #t
          (let* ((pt (car b))
                 (gx (+ px (car pt)))
                 (gy (+ py (cadr pt))))
            (if (grid-cell-occupied? gx gy)
                #f
                (check-blocks (cdr b))))))
    (check-blocks blocks)))

;; Проекция фигуры на дно (Ghost Piece)
(define (calc-ghost-y)
  (define (go gy)
    (if (valid-placement? *piece-type* *piece-rot* *piece-x* (- gy 1))
        (go (- gy 1))
        gy))
  (go *piece-y*))

;; ——— Сгорание линий и начисление очков ———
(define (line-full? y)
  (define (check x)
    (if (= x 10) #t
        (if (table-has? *grid* (+ (* y 10) x))
            (check (+ x 1))
            #f)))
  (check 0))

(define (find-full-lines)
  (define (scan y acc)
    (if (= y 20)
        acc
        (if (line-full? y)
            (scan (+ y 1) (cons y acc))
            (scan (+ y 1) acc))))
  (scan 0 '()))

(define (remove-line! target-y)
  (dotimes (x 10)
    (table-remove! *grid* (+ (* target-y 10) x)))
  (let ((y (+ target-y 1)))
    (while (< y 20)
      (dotimes (x 10)
        (let ((cell (table-get *grid* (+ (* y 10) x) #f)))
          (table-remove! *grid* (+ (* y 10) x))
          (if cell
              (table-set! *grid* (+ (* (- y 1) 10) x) cell))))
      (set! y (+ y 1)))))

(define (clear-full-lines!)
  (let ((lines (find-full-lines)))
    (if (not (null? lines))
        (begin
          ;; 1. Эффект взрыва частиц вдоль каждой сгоревшей линии
          (for-each
           (lambda (y)
             (let ((wy (- y 9.5)))
               (dotimes (x 10)
                 (let ((cell (table-get *grid* (+ (* y 10) x) (list 0.9 0.9 1.0))))
                   (part-burst! *parts* 12 (- x 4.5) wy 0.0 0.0 4.0 0.0 4.5 1.5 0.75 0.20
                                (car cell) (cadr cell) (caddr cell))))))
           lines)

          ;; 2. Удаление линий и смещение поля
          (for-each (lambda (y) (remove-line! y)) lines)
          (let ((n (length lines)))
            (set! *lines* (+ *lines* n))
            (set! *level* (+ 1 (floor (/ *lines* 10))))
            (let ((bonus (cond ((= n 1) 100) ((= n 2) 300) ((= n 3) 500) (else 800))))
              (set! *score* (+ *score* (* bonus *level*))))
            (if (>= n 4)
                (play-sfx 'tet-four 1.0)
                (play-sfx 'tet-line 0.9)))))))

;; Фиксация упавшей фигуры
(define (lock-piece!)
  (let ((blocks (piece-blocks *piece-type* *piece-rot*))
        (col (piece-color *piece-type*)))
    (for-each
     (lambda (pt)
       (let ((gx (+ *piece-x* (car pt)))
             (gy (+ *piece-y* (cadr pt))))
         (if (and (>= gx 0) (< gx 10) (>= gy 0) (< gy 20))
             (table-set! *grid* (+ (* gy 10) gx) col))))
     blocks))
  (play-sfx 'tet-drop 0.8)
  (clear-full-lines!)
  ;; Спавн следующей фигуры
  (set! *piece-type* *next-type*)
  (set! *next-type* (random-piece))
  (set! *piece-rot* 0)
  (set! *piece-x* 3)
  (set! *piece-y* 18)
  (if (not (valid-placement? *piece-type* *piece-rot* *piece-x* *piece-y*))
      (begin
        (set! *game-over* #t)
        (play-sfx 'tet-over 1.0))))

;; ——— Сброс игры (Restart) ———
(define (restart-game!)
  (table-clear! *grid*)
  (part-clear! *parts*)
  (set! *score* 0)
  (set! *lines* 0)
  (set! *level* 1)
  (set! *game-over* #f)
  (set! *piece-type* (random-piece))
  (set! *next-type* (random-piece))
  (set! *piece-rot* 0)
  (set! *piece-x* 3)
  (set! *piece-y* 18)
  (set! *fall-timer* 0.0))

;; ——— Обновление (Update dt) ———
(define (update dt)
  (poll-input)

  ;; Клавиша R / К перезапускает игру В ЛЮБОЙ МОМЕНТ!
  (if (or (key-pressed? "r") (key-pressed? "R") (key-pressed? "к") (key-pressed? "К"))
      (restart-game!))

  ;; Симуляция физики частиц (осколки падают вниз)
  (if (> (part-n *parts*) 0)
      (part-step! *parts* dt 0.0 -16.0 0.0 0.94))

  (if (not *game-over*)
      (begin
        ;; Управление перемещением: A / D или Стрелки влево / вправо
        (if (or (key-pressed? "a") (key-pressed? "left"))
            (if (valid-placement? *piece-type* *piece-rot* (- *piece-x* 1) *piece-y*)
                (begin (set! *piece-x* (- *piece-x* 1)) (play-sfx 'tet-move 0.6))))
        (if (or (key-pressed? "d") (key-pressed? "right"))
            (if (valid-placement? *piece-type* *piece-rot* (+ *piece-x* 1) *piece-y*)
                (begin (set! *piece-x* (+ *piece-x* 1)) (play-sfx 'tet-move 0.6))))

        ;; Вращение: W или Стрелка вверх (с wall-kick)
        (if (or (key-pressed? "w") (key-pressed? "up"))
            (let ((new-rot (mod (+ *piece-rot* 1) 4)))
              (cond
                ((valid-placement? *piece-type* new-rot *piece-x* *piece-y*)
                 (set! *piece-rot* new-rot) (play-sfx 'tet-rot 0.8))
                ((valid-placement? *piece-type* new-rot (- *piece-x* 1) *piece-y*)
                 (set! *piece-rot* new-rot) (set! *piece-x* (- *piece-x* 1)) (play-sfx 'tet-rot 0.8))
                ((valid-placement? *piece-type* new-rot (+ *piece-x* 1) *piece-y*)
                 (set! *piece-rot* new-rot) (set! *piece-x* (+ *piece-x* 1)) (play-sfx 'tet-rot 0.8)))))

        ;; Мгновенный сброс (Hard Drop): Пробел (в окне браузера и в терминале)
        (if (or (key-pressed? " ") (key-pressed? "space"))
            (begin
              (set! *piece-y* (calc-ghost-y))
              (lock-piece!)
              (set! *fall-timer* 0.0)))

        ;; Гравитация и ускоренный спуск (Soft Drop): S или Стрелка вниз
        (let* ((soft-down (or (key-down? "s") (key-down? "down")))
               (speed-factor (if soft-down 0.04 (max 0.1 (- 0.65 (* *level* 0.05)))))
               (new-timer (+ *fall-timer* dt)))
          (if (>= new-timer speed-factor)
              (begin
                (set! *fall-timer* 0.0)
                (if (valid-placement? *piece-type* *piece-rot* *piece-x* (- *piece-y* 1))
                    (set! *piece-y* (- *piece-y* 1))
                    (lock-piece!)))
              (set! *fall-timer* new-timer))))))

;; ——— 3D Рендеринг ———

;; Сборка инстансов всех блоков колодца
(define (collect-instances t)
  (let* ((insts '())
         (acol (piece-color *piece-type*))
         (ar (car acol))
         (ag (cadr acol))
         (ab (caddr acol))
         (px (+ *piece-x* 1.5))
         (py (+ *piece-y* 1.0)))

    ;; 1. Статичные блоки в стакане:
    ;; Они освещаются падающей фигурой в реальном времени!
    (for-each
     (lambda (entry)
       (let* ((key (string->number (car entry)))
              (col (cadr entry))
              (gx (mod key 10))
              (gy (floor (/ key 10)))
              (wx (- gx 4.5))
              (wy (- gy 9.5))
              (dx (- gx px))
              (dy (- gy py))
              (d2 (+ (* dx dx) (* dy dy)))
              (illum (if (< d2 36.0) (* (- 1.0 (/ (sqrt d2) 6.0)) 0.55) 0.0))
              (cr (min 1.0 (+ (car col) (* ar illum))))
              (cg (min 1.0 (+ (cadr col) (* ag illum))))
              (cb (min 1.0 (+ (caddr col) (* ab illum)))))
         (set! insts (append (inst-row wx wy 0.0 1.0 0.0 cr cg cb) insts))))
     (table-entries *grid*))

    ;; 2. Призрачная фигура на дне (Ghost Piece)
    (let ((ghost-y (calc-ghost-y))
          (blocks (piece-blocks *piece-type* *piece-rot*))
          (col (piece-color *piece-type*)))
      (for-each
       (lambda (pt)
         (let ((wx (- (+ *piece-x* (car pt)) 4.5))
               (wy (- (+ ghost-y (cadr pt)) 9.5)))
           (set! insts (append (inst-row wx wy -0.05 0.9 0.0
                                         (* (car col) 0.3)
                                         (* (cadr col) 0.3)
                                         (* (caddr col) 0.3)) insts))))
       blocks))

    ;; 3. Активная падающая фигура: мощная неоновая эмиссия света
    (let* ((blocks (piece-blocks *piece-type* *piece-rot*))
           (glow (+ 1.15 (* 0.20 (sin (* t 8.0)))))
           (cr (min 1.0 (* ar glow)))
           (cg (min 1.0 (* ag glow)))
           (cb (min 1.0 (* ab glow))))
      (for-each
       (lambda (pt)
         (let ((wx (- (+ *piece-x* (car pt)) 4.5))
               (wy (- (+ *piece-y* (cadr pt)) 9.5)))
           (set! insts (append (inst-row wx wy 0.0 1.0 0.0 cr cg cb) insts))))
       blocks))

    insts))

;; Сборка инстансов рамки шахты
(define (collect-frame-instances)
  (let ((insts '()))
    (dotimes (i 21)
      (set! insts (append (inst-row -5.5 (- i 10.0) 0.0 1.0 0.0 0.12 0.16 0.28) insts))
      (set! insts (append (inst-row 5.5 (- i 10.0) 0.0 1.0 0.0 0.12 0.16 0.28) insts)))
    (let ((x -5))
      (while (<= x 5)
        (set! insts (append (inst-row x -10.5 0.0 1.0 0.0 0.12 0.16 0.28) insts))
        (set! x (+ x 1))))
    insts))

(define (draw-3d-scene t)
  ;; Настройка камеры: ракурс сверху-справа дает идеальный 3D-объем граней
  (let* ((eye (list 1.6 1.8 23.5))
         (target (list 0.0 0.0 0.0))
         (mvp (camera-mvp eye target 0.85))
         (insts (collect-instances t))
         (frame-insts (collect-frame-instances))
         (bg-model (mat4-mul (mat4-mul (mat-translate 0.0 0.0 -0.55)
                                       (mat-scale 0.55 1.05 1.0))
                             (mat-rotate-x 1.5707963)))
         (acol (piece-color *piece-type*))
         (ar (car acol))
         (ag (cadr acol))
         (ab (caddr acol))
         (px (+ *piece-x* 1.5))
         (py (+ *piece-y* 1.0)))

    ;; 1. Задняя панель колодца: глубокий темный космос + тонкая неоновая сетка ячеек
    (draw-mesh *mesh-bg* bg-model mvp (list 0.02 0.025 0.045))
    (draw-wire-lines *bg-wire* bg-model mvp (list 0.06 0.14 0.24))

    ;; 2. Световой ореол на заднике под падающей фигурой (свет скользит вместе с блоками)
    (let* ((glow-x (- px 4.5))
           (glow-y (- py 9.5))
           (glow-m (mat4-mul (mat-translate glow-x glow-y -0.52)
                             (mat-scale 0.25 0.25 1.0))))
      (draw-wire-lines *bg-wire* glow-m mvp (list (* ar 0.6) (* ag 0.6) (* ab 0.6))))

    ;; 3. Рамка шахты и блоки Тетриса (WebGPU аппаратный батчинг)
    (draw-batch-tex *mesh-wall* *uv-cube* *tex-panel* frame-insts mvp)
    (draw-batch-tex *mesh-block* *uv-cube* *tex-panel* insts mvp)

    ;; 4. Отрисовка взрывающихся неоновых частиц (осколки сгоревших линий)
    (if (> (part-n *parts*) 0)
        (let* ((n (part-dump! *parts* *parts-out*))
               (p-insts '()))
          (define (collect-p k)
            (if (< k n)
                (let* ((i (* k 8))
                       (px (f32ref *parts-out* i))
                       (py (f32ref *parts-out* (+ i 1)))
                       (pz (f32ref *parts-out* (+ i 2)))
                       (sz (f32ref *parts-out* (+ i 3)))
                       (cr (f32ref *parts-out* (+ i 4)))
                       (cg (f32ref *parts-out* (+ i 5)))
                       (cb (f32ref *parts-out* (+ i 6)))
                       (lf (f32ref *parts-out* (+ i 7)))
                       (sc (* sz (+ 0.35 (* 0.65 lf)))))
                  (set! p-insts (append (inst-row px py pz sc 0.0 cr cg cb) p-insts))
                  (collect-p (+ k 1)))))
          (collect-p 0)
          (if (not (null? p-insts))
              (draw-batch *part-mesh* p-insts mvp))))

    ;; 5. Вращающееся 3D-превью следующей фигуры («NEXT») сбоку
    (let* ((next-col (piece-color *next-type*))
           (next-blocks (piece-blocks *next-type* 0))
           (next-insts '()))
      (for-each
       (lambda (pt)
         (let ((nx (- (car pt) 1.0))
               (ny (- (cadr pt) 0.5)))
           (set! next-insts (append (inst-row nx ny 0.0 0.8 (* t 1.5)
                                             (car next-col) (cadr next-col) (caddr next-col))
                                   next-insts))))
       next-blocks)
      (let ((next-model (mat-translate 9.5 5.5 0.0)))
        (draw-batch-tex *mesh-block* *uv-cube* *tex-panel* next-insts (mat4-mul mvp next-model))))))

;; ——— 2D Интерфейс (HUD) с КРУПНЫМ шрифтом ———
(define (draw-hud)
  ;; Название
  (draw-text "NEON TETRIS 3D" 14 8 2 (list 0.1 0.95 1.0))

  ;; КРУПНЫЕ показатели: очки, линии, уровень
  (draw-text "SCORE" 14 34 1 (list 0.55 0.65 0.80))
  (draw-text (number->string *score*) 14 46 2 (list 1.0 1.0 1.0))

  (draw-text "LINES" 14 70 1 (list 0.55 0.65 0.80))
  (draw-text (number->string *lines*) 14 82 2 (list 0.35 0.90 1.0))

  (draw-text "LEVEL" 14 106 1 (list 0.55 0.65 0.80))
  (draw-text (number->string *level*) 14 118 2 (list 1.0 0.85 0.20))

  ;; Окно NEXT сбоку (крупный заголовок)
  (draw-text "NEXT" (- *screen-w* 76) 36 2 (list 0.8 0.85 1.0))

  ;; Подсказки управления внизу
  (draw-text "A/D: MOVE   W: ROTATE   S: SOFT DROP   SPACE: HARD DROP   R: RESTART"
             (/ (- *screen-w* (text-width "A/D: MOVE   W: ROTATE   S: SOFT DROP   SPACE: HARD DROP   R: RESTART" 1)) 2)
             (- *screen-h* 14) 1 (list 0.5 0.6 0.7))

  ;; Экран завершения
  (if *game-over*
      (begin
        (draw-text "GAME OVER"
                   (/ (- *screen-w* (text-width "GAME OVER" 3)) 2)
                   (- (/ *screen-h* 2) 20) 3 (list 1.0 0.25 0.25))
        (draw-text "PRESS R TO RESTART"
                   (/ (- *screen-w* (text-width "PRESS R TO RESTART" 1)) 2)
                   (+ (/ *screen-h* 2) 22) 1 (list 1.0 1.0 1.0)))))

;; ——— Макрос кадра 3D ———
(defmacro with-frame3d body
  `(begin (gpu3d-clear 0.015 0.018 0.035) ,@body (gpu-present)))

;; ——— Главный цикл отрисовки кадра ———
(define (draw)
  (let ((t (time-now)))
    (with-frame3d
     (draw-3d-scene t)
     (draw-hud))))
