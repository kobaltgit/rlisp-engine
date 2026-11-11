;; tetris.lisp — «НЕОНОВЫЙ 3D-ТЕТРИС» (rlisp-engine)
;; Аппаратный 3D WebGPU, сочные неоновые блоки, физические точечные источники света,
;; яркие вспышки сгорания линий, крупный четкий HUD, мгновенный Hard Drop (Space) и Restart (R).

;; ——— Синхронизация разрешения экрана (фикс сброса при reload) ———
(define (sync-screen-size!)
  (let ((ws (window-size)))
    (set! *screen-w* (car ws))
    (set! *screen-h* (cadr ws))))
(sync-screen-size!)

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

;; ——— Вспышка при уничтожении слоев ———
(defpersistent *flash-timer* 0.0)
(defpersistent *flash-y* 0.0)
(defpersistent *flash-col* (list 1.0 1.0 1.0))

;; ——— Система частиц при взрыве сгорающих линий ———
(define *parts* (part-new))
(define *parts-out* (make-f32vec 4096 0.0))
(define *part-mesh* (mesh-cube 0.22))

;; ——— Процедурные текстуры кристаллического неона и стоек шахты ———
;; Чистый кристалл: яркая внешняя фаска 1.0, кристаллическое тело 0.85..0.96, без грязи
(define (make-neon-crystal-tex n)
  (let ((buf (tex-canvas n n)))
    (dotimes (y n)
      (dotimes (x n)
        (let* ((edge (or (< x 2) (< y 2) (>= x (- n 2)) (>= y (- n 2))))
               (bevel (or (< x 5) (< y 5) (>= x (- n 5)) (>= y (- n 5))))
               (diag (or (= x y) (= (+ x y) (- n 1)))))
          (if edge
              (tex-px! buf n x y 1.0 1.0 1.0)
              (if bevel
                  (tex-px! buf n x y 0.95 0.95 0.98)
                  (if diag
                      (tex-px! buf n x y 0.92 0.92 0.95)
                      (tex-px! buf n x y 0.85 0.86 0.90)))))))
    (tex-fin buf n n)))

;; Стойки шахты колодца с акцентными светоотражающими канавками
(define (make-neon-wall-tex n)
  (let ((buf (tex-canvas n n)))
    (dotimes (y n)
      (dotimes (x n)
        (let* ((groove (or (< (mod y 8) 2) (< (mod x 8) 2)))
               (edge (or (< x 2) (>= x (- n 2)))))
          (if edge
              (tex-px! buf n x y 0.80 0.90 1.0)
              (if groove
                  (tex-px! buf n x y 0.45 0.55 0.70)
                  (tex-px! buf n x y 0.20 0.25 0.35))))))
    (tex-fin buf n n)))

(define *tex-crystal* (make-neon-crystal-tex 64))
(define *tex-wall*    (make-neon-wall-tex 64))

;; ——— Геометрия 3D ———
(define *mesh-block* (mesh-cube 0.47))     ;; кубик 0.94 со скошенной фаской
(define *mesh-wall*  (mesh-cube 0.50))     ;; стойки колодца
(define *mesh-bg*    (mesh-plane 22 10))   ;; задняя стенка
(define *bg-wire*    (list (mesh-pos *mesh-bg*) (mesh-plane-lines 10)))
(define *uv-cube*    (mesh-cube-uv))

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
          ;; 1. Активация мощной неоновой вспышки
          (let* ((first-y (car lines))
                 (sample-cell (table-get *grid* (+ (* first-y 10) 4) (list 0.2 0.9 1.0))))
            (set! *flash-timer* 0.45)
            (set! *flash-y* (- first-y 9.5))
            (set! *flash-col* sample-cell))

          ;; 2. Взрыв 160+ неоновых частиц вдоль каждой сгоревшей линии
          (for-each
           (lambda (y)
             (let ((wy (- y 9.5)))
               (dotimes (x 10)
                 (let ((cell (table-get *grid* (+ (* y 10) x) (list 0.9 0.9 1.0))))
                   (part-burst! *parts* 16 (- x 4.5) wy 0.0 0.0 4.0 0.0 6.0 2.0 0.85 0.22
                                (car cell) (cadr cell) (caddr cell))))))
           lines)

          ;; 3. Удаление линий и смещение поля
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
  (set! *flash-timer* 0.0)
  (set! *piece-type* (random-piece))
  (set! *next-type* (random-piece))
  (set! *piece-rot* 0)
  (set! *piece-x* 3)
  (set! *piece-y* 18)
  (set! *fall-timer* 0.0))

;; ——— Обновление (Update dt) ———
(define (update dt)
  (poll-input)

  ;; Клавиша R / К перезапускает игру В ЛЮБОЙ МОМЕНТ с мгновенным сбросом буфера
  (if (or (key-pressed? "r") (key-down? "r") (key-pressed? "R") (key-down? "R")
          (key-pressed? "к") (key-down? "к") (key-pressed? "К") (key-down? "К"))
      (begin
        (table-set! *keys* "r" -999.0)
        (table-set! *keys* "R" -999.0)
        (table-set! *keys* "к" -999.0)
        (table-set! *keys* "К" -999.0)
        (restart-game!)))

  ;; Таймер неоновой вспышки сгорания линий
  (if (> *flash-timer* 0.0)
      (set! *flash-timer* (max 0.0 (- *flash-timer* dt))))

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

        ;; Мгновенный сброс (Hard Drop): Пробел (в браузере и в терминале)
        (if (or (key-pressed? " ") (key-pressed? "space") (key-down? " ") (key-down? "space"))
            (begin
              (table-set! *keys* "space" -999.0)
              (table-set! *keys* " " -999.0)
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

;; Сборка инстансов всех блоков колодца с динамическим неоновым освещением
(define (collect-instances t)
  (let* ((insts '())
         (acol (piece-color *piece-type*))
         (ar (car acol))
         (ag (cadr acol))
         (ab (caddr acol))
         (px (+ *piece-x* 1.5))
         (py (+ *piece-y* 1.0))
         (flash? (> *flash-timer* 0.0))
         (fk (if flash? (/ *flash-timer* 0.45) 0.0))
         (fr (if flash? (* (car *flash-col*) fk) 0.0))
         (fg (if flash? (* (cadr *flash-col*) fk) 0.0))
         (fb (if flash? (* (caddr *flash-col*) fk) 0.0)))

    ;; 1. Статичные блоки в стакане: физически освещаются падающей фигурой и вспышками
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
              (illum (if (< d2 49.0) (* (- 1.0 (/ (sqrt d2) 7.0)) 0.70) 0.0))
              (flash-illum (if flash?
                               (let ((fdy (abs (- wy *flash-y*))))
                                 (max 0.0 (- 1.0 (/ fdy 6.0))))
                               0.0))
              (cr (min 1.0 (+ (car col) (* ar illum) (* fr flash-illum))))
              (cg (min 1.0 (+ (cadr col) (* ag illum) (* fg flash-illum))))
              (cb (min 1.0 (+ (caddr col) (* ab illum) (* fb flash-illum)))))
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
                                         (* (car col) 0.32)
                                         (* (cadr col) 0.32)
                                         (* (caddr col) 0.32)) insts))))
       blocks))

    ;; 3. Активная падающая фигура: мощная неоновая эмиссия света
    (let* ((blocks (piece-blocks *piece-type* *piece-rot*))
           (glow (+ 1.20 (* 0.25 (sin (* t 8.0)))))
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

;; Сборка инстансов рамки шахты с динамическим отражением неонового света
(define (collect-frame-instances)
  (let* ((insts '())
         (acol (piece-color *piece-type*))
         (ar (car acol))
         (ag (cadr acol))
         (ab (caddr acol))
         (px (+ *piece-x* 1.5))
         (py (+ *piece-y* 1.0))
         (p-wx (- px 4.5))
         (p-wy (- py 9.5))
         (flash? (> *flash-timer* 0.0))
         (fk (if flash? (/ *flash-timer* 0.45) 0.0))
         (fr (if flash? (* (car *flash-col*) fk) 0.0))
         (fg (if flash? (* (cadr *flash-col*) fk) 0.0))
         (fb (if flash? (* (caddr *flash-col*) fk) 0.0)))

    ;; Левая и правая вертикальные стойки
    (dotimes (i 21)
      (let* ((wy (- i 10.0))
             ;; Освещение левой стойки
             (ldx (- -5.5 p-wx))
             (ldy (- wy p-wy))
             (ld2 (+ (* ldx ldx) (* ldy ldy)))
             (l-ill (if (< ld2 64.0) (* (- 1.0 (/ (sqrt ld2) 8.0)) 0.75) 0.0))
             (lf-ill (if flash? (max 0.0 (- 1.0 (/ (abs (- wy *flash-y*)) 7.0))) 0.0))
             (lcr (min 1.0 (+ 0.12 (* ar l-ill) (* fr lf-ill 1.2))))
             (lcg (min 1.0 (+ 0.16 (* ag l-ill) (* fg lf-ill 1.2))))
             (lcb (min 1.0 (+ 0.28 (* ab l-ill) (* fb lf-ill 1.2))))

             ;; Освещение правой стойки
             (rdx (- 5.5 p-wx))
             (rdy (- wy p-wy))
             (rd2 (+ (* rdx rdx) (* rdy rdy)))
             (r-ill (if (< rd2 64.0) (* (- 1.0 (/ (sqrt rd2) 8.0)) 0.75) 0.0))
             (rf-ill (if flash? (max 0.0 (- 1.0 (/ (abs (- wy *flash-y*)) 7.0))) 0.0))
             (rcr (min 1.0 (+ 0.12 (* ar r-ill) (* fr rf-ill 1.2))))
             (rcg (min 1.0 (+ 0.16 (* ag r-ill) (* fg rf-ill 1.2))))
             (rcb (min 1.0 (+ 0.28 (* ab r-ill) (* fb rf-ill 1.2)))))

        (set! insts (append (inst-row -5.5 wy 0.0 1.0 0.0 lcr lcg lcb) insts))
        (set! insts (append (inst-row  5.5 wy 0.0 1.0 0.0 rcr rcg rcb) insts))))

    ;; Нижнее основание шахты
    (let ((x -5))
      (while (<= x 5)
        (let* ((wx x)
               (wy -10.5)
               (bdx (- wx p-wx))
               (bdy (- wy p-wy))
               (bd2 (+ (* bdx bdx) (* bdy bdy)))
               (b-ill (if (< bd2 64.0) (* (- 1.0 (/ (sqrt bd2) 8.0)) 0.75) 0.0))
               (bf-ill (if flash? (max 0.0 (- 1.0 (/ (abs (- wy *flash-y*)) 7.0))) 0.0))
               (bcr (min 1.0 (+ 0.12 (* ar b-ill) (* fr bf-ill 1.2))))
               (bcg (min 1.0 (+ 0.16 (* ag b-ill) (* fg bf-ill 1.2))))
               (bcb (min 1.0 (+ 0.28 (* ab b-ill) (* fb bf-ill 1.2)))))
          (set! insts (append (inst-row wx wy 0.0 1.0 0.0 bcr bcg bcb) insts))
          (set! x (+ x 1)))))
    insts))

(define (draw-3d-scene t)
  ;; Настройка конвейера освещения и камеры
  (gpu3d-view 1.6 1.8 23.5)
  (gpu3d-light 0.25 0.55 0.80 0.65 0.45 0.60 32.0)
  (gpu3d-clear-lights)

  (let* ((eye (list 1.6 1.8 23.5))
         (target (list 0.0 0.0 0.0))
         (mvp (camera-mvp eye target 0.85))
         (acol (piece-color *piece-type*))
         (ar (car acol))
         (ag (cadr acol))
         (ab (caddr acol))
         (px (+ *piece-x* 1.5))
         (py (+ *piece-y* 1.0))
         (p-wx (- px 4.5))
         (p-wy (- py 9.5))
         (insts (collect-instances t))
         (frame-insts (collect-frame-instances))
         (bg-model (mat4-mul (mat4-mul (mat-translate 0.0 0.0 -0.55)
                                       (mat-scale 0.55 1.05 1.0))
                             (mat-rotate-x 1.5707963))))

    ;; 1. Добавление динамического точечного источника света от падающей фигуры
    (gpu3d-add-point-light p-wx p-wy 1.2 (* ar 2.2) (* ag 2.2) (* ab 2.2) 9.5 1.2)

    ;; 2. Добавление сверхяркого точечного источника при сгорании линии
    (if (> *flash-timer* 0.0)
        (let ((f (* (/ *flash-timer* 0.45) 3.5))
              (fc *flash-col*))
          (gpu3d-add-point-light 0.0 *flash-y* 1.6 (* (car fc) f) (* (cadr fc) f) (* (caddr fc) f) 18.0 1.6)))

    ;; 3. Задняя панель колодца: глубокий темный космос + тонкая неоновая сетка ячеек
    (draw-mesh *mesh-bg* bg-model mvp (list 0.02 0.025 0.045))
    (let ((wire-col (if (> *flash-timer* 0.0)
                        (let ((f (* (/ *flash-timer* 0.45) 1.8)))
                          (list (min 1.0 (+ 0.06 (* (car *flash-col*) f)))
                                (min 1.0 (+ 0.14 (* (cadr *flash-col*) f)))
                                (min 1.0 (+ 0.24 (* (caddr *flash-col*) f)))))
                        (list 0.06 0.14 0.24))))
      (draw-wire-lines *bg-wire* bg-model mvp wire-col))

    ;; 6. Рамка шахты (металл/канавки) и блоки Тетриса (кристаллический неон)
    (draw-batch-tex *mesh-wall* *uv-cube* *tex-wall* frame-insts mvp)
    (draw-batch-tex *mesh-block* *uv-cube* *tex-crystal* insts mvp)

    ;; 7. Отрисовка взрывающихся неоновых частиц (осколки сгоревших линий)
    (if (> (part-n *parts*) 0)
        (let* ((n (part-dump! *parts* *parts-out*))
               (p-insts '()))
          (define (collect-p k)
            (if (< k n)
                (let* ((i (* k 8))
                       (ptx (f32ref *parts-out* i))
                       (pty (f32ref *parts-out* (+ i 1)))
                       (ptz (f32ref *parts-out* (+ i 2)))
                       (sz (f32ref *parts-out* (+ i 3)))
                       (cr (f32ref *parts-out* (+ i 4)))
                       (cg (f32ref *parts-out* (+ i 5)))
                       (cb (f32ref *parts-out* (+ i 6)))
                       (lf (f32ref *parts-out* (+ i 7)))
                       (sc (* sz (+ 0.35 (* 0.65 lf)))))
                  (set! p-insts (append (inst-row ptx pty ptz sc 0.0 cr cg cb) p-insts))
                  (collect-p (+ k 1)))))
          (collect-p 0)
          (if (not (null? p-insts))
              (draw-batch *part-mesh* p-insts mvp))))

    ;; 8. Вращающееся 3D-превью следующей фигуры («NEXT») сбоку
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
        (draw-batch-tex *mesh-block* *uv-cube* *tex-crystal* next-insts (mat4-mul mvp next-model))))))

;; ——— 2D Интерфейс (HUD) с КРУПНЫМ шрифтом и адаптацией под разрешение ———
(define (draw-hud)
  (let* ((large? (>= *screen-w* 500))
         (s-title (if large? 3 2))
         (s-lbl   (if large? 2 1))
         (s-val   (if large? 3 2))
         (pad-x   (if large? 20 12))
         (y-sc    (if large? 58 34))
         (y-sc-v  (if large? 76 46))
         (y-ln    (if large? 116 70))
         (y-ln-v  (if large? 134 82))
         (y-lv    (if large? 174 106))
         (y-lv-v  (if large? 192 118))
         (next-x  (- *screen-w* (if large? 110 76)))
         (next-y  (if large? 58 36)))

    ;; Заголовок
    (draw-text "NEON TETRIS 3D" pad-x (if large? 16 8) s-title (list 0.10 0.95 1.00))

    ;; КРУПНЫЕ показатели: очки, линии, уровень
    (draw-text "SCORE" pad-x y-sc s-lbl (list 0.55 0.65 0.85))
    (draw-text (number->string *score*) pad-x y-sc-v s-val (list 1.00 1.00 1.00))

    (draw-text "LINES" pad-x y-ln s-lbl (list 0.55 0.65 0.85))
    (draw-text (number->string *lines*) pad-x y-ln-v s-val (list 0.35 0.90 1.00))

    (draw-text "LEVEL" pad-x y-lv s-lbl (list 0.55 0.65 0.85))
    (draw-text (number->string *level*) pad-x y-lv-v s-val (list 1.00 0.85 0.20))

    ;; Окно NEXT сбоку (крупный заголовок)
    (draw-text "NEXT" next-x next-y s-lbl (list 0.85 0.90 1.00))

    ;; Подсказки управления внизу по центру экрана
    (let ((hint "A/D: MOVE   W: ROTATE   S: SOFT DROP   SPACE: HARD DROP   R: RESTART"))
      (draw-text hint
                 (/ (- *screen-w* (text-width hint 1)) 2)
                 (- *screen-h* (if large? 20 14)) 1 (list 0.55 0.65 0.75)))

    ;; Экран завершения
    (if *game-over*
        (begin
          (draw-text "GAME OVER"
                     (/ (- *screen-w* (text-width "GAME OVER" (if large? 4 3))) 2)
                     (- (/ *screen-h* 2) (if large? 30 20)) (if large? 4 3) (list 1.00 0.25 0.25))
          (draw-text "PRESS R TO RESTART"
                     (/ (- *screen-w* (text-width "PRESS R TO RESTART" (if large? 2 1))) 2)
                     (+ (/ *screen-h* 2) (if large? 28 22)) (if large? 2 1) (list 1.00 1.00 1.00))))))

;; ——— Макрос кадра 3D ———
(defmacro with-frame3d body
  `(begin (gpu3d-clear 0.015 0.018 0.035) ,@body (gpu-present)))

;; ——— Главный цикл отрисовки кадра ———
(define (draw)
  (sync-screen-size!)
  (let ((t (time-now)))
    (with-frame3d
     (draw-3d-scene t)
     (draw-hud))))
