;; ruiner.lisp — «РУИНЕР» (этап 28): DOOM-подобный лабиринтный шутер на нативах
;; движка. Старая школа: сетка 24×24, хитскан-выстрел, демоны-билборды,
;; аптечки/патроны, портал на следующий уровень. Всё на Lisp: AI, коллизии,
;; LOS, спрайты на tex-canvas, чиптюн-звук. Управление: W/S — вперёд/назад,
;; A/D — поворот, Q/E — стрейф, ПРОБЕЛ — огонь, R — рестарт после гибели.

(load "audio.lisp")

(defpersistent *t* 0.0)
(defpersistent *frame* 0)
(defpersistent *fps-last* 0)
(defpersistent *fps-val* 0.0)
(defpersistent *fps-clock* 0)
(defpersistent *score* 0)
(defpersistent *kills* 0)
(defpersistent *level* 1)
(defpersistent *hp* 100)
(defpersistent *ammo* 30)
(defpersistent *state* 'play)          ; play / dead
(defpersistent *hurt-flash* 0.0)
(defpersistent *muzzle* 0.0)
(defpersistent *msg* "")
(defpersistent *msg-t* 0.0)

;; ——— параметры мира ———
(define *N* 24)                 ; сетка 24×24
(define *wall-h* 2.6)
(define *grid* (make-f32vec (* *N* *N*) 0.0))  ; 1.0 = стена
(define *rooms* '())
(define (cddddr x) (cdr (cdr (cdr (cdr x)))))
(define (cdddddr x) (cdr (cdr (cdr (cdr (cdr x))))))
(define (cddddddr x) (cdr (cdr (cdr (cdr (cdr (cdr x)))))))
;; враги — cons-пары (set-car! работает только с парами, не с Vec-списками)
(define (cons7 a b c d e f g)
  (cons a (cons b (cons c (cons d (cons e (cons f (cons g '()))))))))


;; ——— клетки ———
(define (g-ix x z) (+ (* z *N*) x))
(define (wall? x z)
  (if (or (< x 0) (>= x *N*) (< z 0) (>= z *N*))
      #t
      (= 1.0 (f32ref *grid* (g-ix (floor x) (floor z))))))

;; ——— генерация уровня: комнаты + L-коридоры (детерминированно от уровня) ———
(define (cell-cx i) (mod (hash-int *level* i 11) 20))
(define (build-level)
  (dotimes (i (* *N* *N*)) (f32set! *grid* i 1.0))
  (set! *rooms* '())
  ;; комнаты: 9 прямоугольников
  (define (go-rooms k)
    (if (>= k 9)
        #f
        (let* ((w (+ 3 (mod (hash-int *level* k 21) 4)))
               (h (+ 3 (mod (hash-int *level* k 31) 4)))
               (x (mod (hash-int *level* k 41) (- *N* w 2)))
               (z (mod (hash-int *level* k 51) (- *N* h 2))))
          (dotimes (i w)
            (dotimes (j h)
              (f32set! *grid* (g-ix (+ x i) (+ z j)) 0.0)))
          (set! *rooms* (cons (list (+ x (/ w 2.0)) (+ z (/ h 2.0)) w h)
                              *rooms*))
          (go-rooms (+ k 1)))))
  (go-rooms 0)
  ;; коридоры: соединяем центры комнат L-путём
  (define (carve x z) (f32set! *grid* (g-ix x z) 0.0))
  (define (corridor a b)
    (let ((ax (floor (car a))) (az (floor (cadr a)))
          (bx (floor (car b))) (bz (floor (cadr b))))
      (define (go-x x) (if (= x bx) #f (begin (carve x az) (go-x (+ x (if (< x bx) 1 -1))))))
      (define (go-z z) (if (= z bz) #f (begin (carve bx z) (go-z (+ z (if (< z bz) 1 -1))))))
      (go-x ax) (go-z az)))
  (define (link k)
    (if (>= k (- (length *rooms*) 1))
        #f
        (begin (corridor (nth *rooms* k) (nth *rooms* (+ k 1)))
               (link (+ k 1)))))
  (link 0)
  (carve 1 1))

;; ——— старт игрока и объектов уровня ———
(defpersistent *px* 1.5)
(defpersistent *pz* 1.5)
(defpersistent *pa* 0.7)
(defpersistent *enemies* '())
(defpersistent *items* '())
(defpersistent *exit* 0)

(define (spawn-level)
  (build-level)
  ;; старт — центр САМОЙ БОЛЬШОЙ комнаты
  (let ((big 0) (bigr (list 12 12)))
    (define (pick rs)
      (if (null? rs)
          #f
          (let ((r (car rs)))
            (let ((a (* (nth r 2) (nth r 3))))
              (if (> a big)
                  (begin (set! big a) (set! bigr r))
                  #f))
            (pick (cdr rs)))))
    (pick *rooms*)
    (set! *px* (car bigr)) (set! *pz* (cadr bigr))
    ;; взгляд: к выходу-порталу
    (set! *pa* (atan (- (cadr (car *rooms*)) *pz*) (- (car (car *rooms*)) *px*))))
  (set! *enemies* '())
  (set! *items* '())
  ;; выход — центр самой дальней комнаты от старта
  (let ((best 0) (bestr (list 12 12)))
    (define (go rs)
      (if (null? rs)
          #f
          (let ((r (car rs)))
            (let ((dx (- (car r) 1.5)) (dz (- (cadr r) 1.5)))
              (let ((d (+ (* dx dx) (* dz dz))))
                (if (> d best)
                    (begin (set! best d) (set! bestr r))
                    #f)
                (go (cdr rs)))))))
    (go *rooms*)
    (set! *exit* (list (car bestr) (cadr bestr))))
  ;; враги: по демону в комнате, кроме стартовой
  (let ((k 0))
    (define (go-e rs)
      (if (null? rs)
          #f
          (let ((r (car rs)))
            (let ((dx (- (car r) 1.5)) (dz (- (cadr r) 1.5)))
              (let ((d (+ (* dx dx) (* dz dz))))
                (if (> d 64.0)
                    (begin
                      (set! *enemies*
                            (cons (cons7 (- (car r) 0.6) (- (cadr r) 0.6)
                                         (+ 26 (* 8 (- *level* 1)))
                                         'idle 0.0
                                         (* 6.28318 (/ (mod (hash-int *level* k 71) 628) 100.0))
                                         0.0)
                                  *enemies*))
                      (set! k (+ k 1)))
                    #f)
                (go-e (cdr rs)))))))
    (go-e *rooms*))
  ;; пикапы: 3 аптечки + 3 коробки патронов в свободных клетках
  (let ((j 0) (n 6))
    (define (spot j k)
      (let ((x (mod (hash-int *level* j k 91) *N*))
            (z (mod (hash-int *level* j k 97) *N*)))
        (if (and (= 0.0 (f32ref *grid* (g-ix x z)))
                 (> (+ (* (- x 1) (- x 1)) (* (- z 1) (- z 1))) 25.0))
            (list (+ x 0.5) (+ z 0.5))
            (if (< k 200) (spot j (+ k 1)) (list 1.5 1.5)))))
    (define (go-i)
      (if (>= j n)
          #f
          (let ((s (spot j 0)))
            (set! *items*
                  (cons (list (car s) (cadr s)
                              (if (< j 3) 'medkit 'ammo))
                        *items*))
            (set! j (+ j 1))
            (go-i))))
    (go-i)))




(define (canvas-fill! buf n r g b a)
  (dotimes (i (* n n))
    (let ((i4 (* i 4)))
      (f32set! buf i4 r)
      (f32set! buf (+ i4 1) g)
      (f32set! buf (+ i4 2) b)
      (f32set! buf (+ i4 3) a))))
(define (px! buf n x y r g b)
  (let ((i4 (* 4 (+ (* y n) x))))
    (f32set! buf i4 r) (f32set! buf (+ i4 1) g)
    (f32set! buf (+ i4 2) b)))
(define (rect! buf n x0 y0 w h r g b)
  (dotimes (j h)
    (dotimes (i w)
      (if (and (>= (+ x0 i) 0) (< (+ x0 i) n) (>= (+ y0 j) 0) (< (+ y0 j) n))
          (px! buf n (+ x0 i) (+ y0 j) r g b)
          #f))))

(define *imp-tex*
  (let ((buf (tex-canvas 24 24)))
    (canvas-fill! buf 24 0.06 0.06 0.08 1.0)
    ;; тело: тёмно-красный
    (rect! buf 24 8 8 8 12 0.55 0.16 0.12)
    ;; плечи
    (rect! buf 24 5 9 14 3 0.5 0.14 0.1)
    ;; рога
    (px! buf 24 7 6 0.7 0.6 0.4) (px! buf 24 16 6 0.7 0.6 0.4)
    (px! buf 24 6 5 0.7 0.6 0.4) (px! buf 24 17 5 0.7 0.6 0.4)
    ;; глаза — жёлтые
    (rect! buf 24 9 10 2 2 1.0 0.85 0.2)
    (rect! buf 24 13 10 2 2 1.0 0.85 0.2)
    ;; пасть
    (rect! buf 24 9 14 6 2 0.2 0.05 0.04)
    (px! buf 24 10 15 1 1 0.9) (px! buf 24 13 15 1 1 0.9)
    ;; лапы
    (rect! buf 24 8 20 3 3 0.35 0.1 0.08)
    (rect! buf 24 13 20 3 3 0.35 0.1 0.08)
    (texture-upload 24 24 buf)))
(define *gore-tex*
  (let ((buf (tex-canvas 24 24)))
    (canvas-fill! buf 24 0 0 0 1.0)
    (rect! buf 24 4 16 16 4 0.35 0.06 0.05)
    (rect! buf 24 7 13 10 3 0.42 0.08 0.06)
    (px! buf 24 5 18 0.3 0.04 0.04) (px! buf 24 18 18 0.3 0.04 0.04)
    (texture-upload 24 24 buf)))
(define *med-tex*
  (let ((buf (tex-canvas 16 16)))
    (canvas-fill! buf 16 0 0 0 1.0)
    (rect! buf 16 2 4 12 9 0.92 0.93 0.95)
    (rect! buf 16 6 5 4 7 0.85 0.1 0.12)
    (rect! buf 16 4 7 8 3 0.85 0.1 0.12)
    (texture-upload 16 16 buf)))
(define *ammo-tex*
  (let ((buf (tex-canvas 16 16)))
    (canvas-fill! buf 16 0 0 0 1.0)
    (rect! buf 16 2 6 12 7 0.75 0.6 0.15)
    (rect! buf 16 2 6 12 2 0.9 0.8 0.4)
    (px! buf 16 5 9 0.2 0.15 0.05) (px! buf 16 10 9 0.2 0.15 0.05)
    (texture-upload 16 16 buf)))
(define *flash-tex*
  (let ((buf (tex-canvas 16 16)))
    (canvas-fill! buf 16 0 0 0 1.0)
    (rect! buf 16 6 6 4 4 1.0 0.9 0.4)
    (rect! buf 16 4 7 8 2 1.0 0.75 0.25)
    (px! buf 16 3 8 1 0.6 0.2) (px! buf 16 12 8 1 0.6 0.2)
    (texture-upload 16 16 buf)))

;; ——— меши уровня: стены merged-мешем, пол/потолок ———
;; ——— меши уровня: стены merged-мешем (4 грани на клетку), пол/потолок ———
;; Строится один раз на уровень: позиции 3/верш, цвета 4/верш, индексы.
(define (build-wall-mesh)
  (let ((pos '()) (vc '()) (idx '()) (vi 0))
    (define (quad x0 y0 z0 x1 y1 z1 x2 y2 z2 x3 y3 z3 r g b)
      ;; 4 вершины против часовой (наружу от клетки)
      (set! pos (append pos (list x0 y0 z0 x1 y1 z1 x2 y2 z2 x3 y3 z3)))
      (dotimes (k 4)
        (set! vc (append vc (list r g b 1.0))))
      (set! idx (append idx (list vi (+ vi 1) (+ vi 2) vi (+ vi 2) (+ vi 3))))
      (set! vi (+ vi 4)))
    (dotimes (i *N*)
      (dotimes (j *N*)
        (if (= 1.0 (f32ref *grid* (g-ix i j)))
            (let* ((h (hash-int i j *level*))
                   (v (+ 0.46 (* 0.14 (/ (mod h 100) 100.0))))
                   (x0 i) (x1 (+ i 1)) (z0 j) (z1 (+ j 1))
                   (h0 0.0) (h1 *wall-h*))
              ;; наружу от клетки: -Z, +Z, -X, +X (внутренние грани видны из коридоров)
              (quad x0 h1 z0 x1 h1 z0 x1 h0 z0 x0 h0 z0 v v v)
              (quad x1 h1 z1 x0 h1 z1 x0 h0 z1 x1 h0 z1 v v v)
              (quad x0 h1 z1 x0 h1 z0 x0 h0 z0 x0 h0 z1 (* v 0.9) v (* v 0.95))
              (quad x1 h1 z0 x1 h1 z1 x1 h0 z1 x1 h0 z0 (* v 0.9) v (* v 0.95)))
            #f)))
    (list (f32-from-list pos) (f32-from-list vc) (f32-from-list idx))))
(define *walls* (list (make-f32vec 1 0) (make-f32vec 1 0) (make-f32vec 1 0)))
(define *wall-uv* (make-f32vec 1 0.0))
(define (rebuild-meshes!)
  (set! *walls* (build-wall-mesh))
  (set! *wall-uv* (make-f32vec (* (/ (f32vec-length (car *walls*)) 3) 2) 0.0)))
(define *floor-mesh*
  (mesh-make (list 0 0 0  *N* 0 0  *N* 0 *N*  0 0 *N*)
             (list 0 2 1  0 1 2   2 1 0  2 0 3)))
(define *floor-vc*
  (f32-from-list (list 0.30 0.28 0.26 1  0.30 0.28 0.26 1
                       0.30 0.28 0.26 1  0.30 0.28 0.26 1)))
(define *ceil-mesh*
  (mesh-make (list 0 *wall-h* 0  0 *wall-h* *N*  *N* *wall-h* *N*  *N* *wall-h* 0)
             (list 0 1 2  0 2 3  2 1 0  3 2 0)))
(define *ceil-vc*
  (f32-from-list (list 0.22 0.22 0.24 1  0.22 0.22 0.24 1
                       0.22 0.22 0.24 1  0.22 0.22 0.24 1)))
(define *ceil-uv* (make-f32vec 8 0.0))
(define *floor-uv* (make-f32vec 8 0.0))

;; враги — cons-пары (set-car! работает только с парами, не с Vec-списками)
(define (cddddr x) (cdr (cdr (cdr (cdr x)))))
(define (cdddddr x) (cdr (cdr (cdr (cdr (cdr x))))))
(define (cddddddr x) (cdr (cdr (cdr (cdr (cdr (cdr x)))))))
(define (cons7 a b c d e f g)
  (cons a (cons b (cons c (cons d (cons e (cons f (cons g '()))))))))
;; мутация элемента списка (враги — 7-элементные списки)
(define (eset! e i v)
  (cond ((= i 0) (set-car! e v))
        ((= i 1) (set-car! (cdr e) v))
        ((= i 2) (set-car! (cddr e) v))
        ((= i 3) (set-car! (cdddr e) v))
        ((= i 4) (set-car! (cddddr e) v))
        ((= i 5) (set-car! (cddddr (cdr e)) v))
        ((= i 6) (set-car! (cddddr (cddr e)) v))))

;; ——— коллизии: скольжение вдоль осей ———
(define (can-stand x z)
  (let ((r 0.22))
    (and (= 0.0 (f32ref *grid* (g-ix (floor (- x r)) (floor (- z r)))))
         (= 0.0 (f32ref *grid* (g-ix (floor (+ x r)) (floor (- z r)))))
         (= 0.0 (f32ref *grid* (g-ix (floor (- x r)) (floor (+ z r)))))
         (= 0.0 (f32ref *grid* (g-ix (floor (+ x r)) (floor (+ z r))))))))
(define (move-actor! xz dx dz)
  ;; возвращает (x z); отдельно по осям — скольжение вдоль стен
  (let* ((x (car xz)) (z (cadr xz))
         (nx (+ x dx)) (nz (+ z dz))
         (rx (if (can-stand nx z) nx x))
         (rz (if (can-stand rx nz) nz z)))
    (list rx rz)))

;; ——— LOS по сетке (шаги по лучу) ———
(define (los? ax az bx bz)
  (let* ((dx (- bx ax)) (dz (- bz az))
         (d (max 0.0001 (sqrt (+ (* dx dx) (* dz dz)))))
         (st 0.35)
         (n (floor (/ d st)))
         (sx (* st (/ dx d))) (sz (* st (/ dz d))))
    (define (go k x z)
      (if (> k n)
          #t
          (if (wall? (floor x) (floor z))
              #f
              (go (+ k 1) (+ x sx) (+ z sz)))))
    (go 0 ax az)))

;; ——— звук ———
(defsound-synth 'shot    'noise  900 0.14 0.001 0.02 0.2 0.09 0.0 0.5)
(defsound-synth 'empty   'square 220 0.05 0.001 0.01 0.1 0.04 0.0 0.5)
(defsound-synth 'hurt    'saw    140 0.18 0.001 0.02 0.4 0.1 -60 0.5)
(defsound-synth 'edie    'saw    90  0.3  0.001 0.04 0.5 0.2 -40 0.5)
(defsound-synth 'pickup  'sine   660 0.09 0.001 0.01 0.3 0.04 300 0.5)
(defsound-synth 'level   'square 440 0.22 0.001 0.02 0.4 0.08 220 0.5)
(defsound-synth 'ebite   'noise  500 0.1  0.001 0.02 0.3 0.06 0.0 0.5)

;; ——— стрельба: хитскан по конусу ———
(define (fire!)
  (if (< *ammo* 1)
      (begin (play-sfx 'empty) (set! *msg* "ПАТРОНЫ КОНЧИЛИСЬ") (set! *msg-t* *t*))
      (let ((dx (cos *pa*)) (dz (sin *pa*)))
        (set! *ammo* (- *ammo* 1))
        (set! *muzzle* *t*)
        (play-sfx 'shot)
        ;; будим демонов с LOS
        (define (wake es)
          (if (null? es)
              #f
              (let ((e (car es)))
                (if (and (los? *px* *pz* (car e) (cadr e))
                         (< (+ (* (- (car e) *px*) (- (car e) *px*))
                              (* (- (cadr e) *pz*) (- (cadr e) *pz*))) 400.0))
                    (begin (eset! e 3 'chase) (wake (cdr es)))
                    (wake (cdr es))))))
        (wake *enemies*)
        ;; хитскан: ближайший в конусе ±0.2 рад
        (let ((best #f) (bestd 1e9))
          (define (go es)
            (if (null? es)
                #f
                (let* ((e (car es))
                       (ex (- (car e) *px*)) (ez (- (cadr e) *pz*))
                       (d (sqrt (+ (* ex ex) (* ez ez)))))
                  (if (and (> d 0.01) (< d 28.0) (< d bestd))
                      (let ((ang (- (atan ez ex) *pa*)))
                        (if (and (< (abs ang) 0.22) (los? *px* *pz* (car e) (cadr e)))
                            (begin (set! best e) (set! bestd d) (go (cdr es)))
                            (go (cdr es))))
                      (go (cdr es))))))
          (go *enemies*)
                  (set! *msg-t* *t*)
          (if best
              (let ((nhp (- (caddr best) (+ 12 (mod (hash-int *frame* 5) 8)))))
                (set-car! (cddr best) nhp)
                (set-car! (cddddr (cddr best)) *t*)   ; hit-flash: таймштамп
                (if (<= nhp 0)
                    (begin
                      (play-sfx 'edie)
                      (set! *kills* (+ *kills* 1))
                      (set! *score* (+ *score* 25))
                      (set! *msg* "ДЕМОН ПОВЕРЖЕН")
                      (set! *msg-t* *t*))
                    #f))
              #f)))))

;; ——— обновление демонов ———
(define (update-enemies! dt)
  (define (go es)
    (if (null? es)
        #f
        (let* ((e (car es))
               (ex (car e)) (ez (cadr e)) (hp (caddr e)) (st (cadddr e))
               (ph (nth e 4)) (hit (nth e 6)))
          (cond ((<= hp 0) #f)
                ((eq? st 'idle)
                 (if (and (los? ex ez *px* *pz*)
                          (< (+ (* (- ex *px*) (- ex *px*)) (* (- ez *pz*) (- ez *pz*)))
                             220.0))
                     (eset! e 3 'chase)
                     #f))
                (else
                 (let* ((dx (- *px* ex)) (dz (- *pz* ez))
                        (d (max 0.001 (sqrt (+ (* dx dx) (* dz dz))))))
                   (if (> d 1.0)
                       (let* ((sp (* 2.1 dt))
                              (np (move-actor! (list ex ez)
                                               (* sp (/ dx d)) (* sp (/ dz d)))))
                         (eset! e 0 (car np))
                         (eset! e 1 (cadr np)))
                       (if (> (- *t* ph) 1.1)
                           (begin
                             (eset! e 4 *t*)
                             (set! *hp* (max 0 (- *hp* (+ 6 (mod (hash-int *frame* 13) 9)))))
                             (set! *hurt-flash* 1.0)
                             (play-sfx 'ebite))
                           #f))
                   (if (not (los? ex ez *px* *pz*))
                       (eset! e 3 'idle)
                       #f))))
          (go (cdr es)))))
  (go *enemies*))

;; ——— пикапы ———
(define (update-items!)
  (define (go it keep)
    (if (null? it)
        (set! *items* (reverse keep))
        (let* ((i (car it))
               (dx (- (car i) *px*)) (dz (- (cadr i) *pz*))
               (d2 (+ (* dx dx) (* dz dz))))
          (if (< d2 0.8)
              (begin
                (if (eq? (caddr i) 'medkit)
                    (begin (set! *hp* (min 100 (+ *hp* 25)))
                           (set! *msg* "АПТЕЧКА +25")
                           (set! *score* (+ *score* 5)))
                    (begin (set! *ammo* (+ *ammo* 12))
                           (set! *msg* "ПАТРОНЫ +12")
                           (set! *score* (+ *score* 5))))
                (play-sfx 'pickup)
                (go (cdr it) keep))
              (go (cdr it) (cons i keep))))))
  (go *items* '()))

;; ——— портал: проверка выхода на следующий уровень ———
(define (check-exit!)
  (let* ((ex (car *exit*)) (ez (cadr *exit*))
         (dx (- ex *px*)) (dz (- ez *pz*))
         (d2 (+ (* dx dx) (* dz dz))))
    (if (< d2 1.2)
        (begin
          (play-sfx 'level)
          (set! *level* (+ *level* 1))
          (set! *score* (+ *score* 100))
          (set! *hp* (min 100 (+ *hp* 15)))
          (set! *msg* "УРОВЕНЬ ПРОЙДЕН")
          (set! *msg-t* *t*)
          (spawn-level)
          (rebuild-meshes!))
        #f)))
(define *exit-tex*
  (let ((buf (tex-canvas 16 16)))
    (canvas-fill! buf 16 0 0 0 1.0)
    (dotimes (y 16)
      (let* ((k (/ y 15.0))
             (r (- 0.55 (* 0.5 k)))
             (g (+ 0.35 (* 0.5 k)))
             (b 0.25))
        (dotimes (x 16)
          (if (and (> x 4) (< x 11) (or (> y 13) (< y 2) (> x 7) (< x 8)))
              (px! buf 16 x y r g b)
              #f))))
    (texture-upload 16 16 buf)))
;; юнит-билборд (двусторонний), точка опоры — низ
(define *bb-mesh*
  (mesh-make (list -0.5 0 0  0.5 0 0  0.5 1 0  -0.5 1 0)
             (list 0 1 2  0 2 3  2 1 0  3 2 0)))
(define *bb-uv* (f32-from-list (list 0 0  1 0  1 1  0 1)))
(define (billboard-yaw) (- 4.71239 *pa*))

;; ——— рендер кадра ———
(define (draw-scene)
  (let* ((eye (list *px* 1.15 *pz*))
         (mvp (camera-mvp eye
                          (list (+ *px* (* 10.0 (cos *pa*))) 1.15
                                (+ *pz* (* 10.0 (sin *pa*))))
                          1.1))
         (by (billboard-yaw)))
    (gpu3d-clear 0.05 0.05 0.06)
    (gpu3d-view (car eye) (cadr eye) (caddr eye))
    (gpu3d-fog 0.05 0.05 0.06 0.085)
    (gpu3d-light 0 1 0 0.9 0.2)
    ;; пол и потолок
    (gpu3d-mesh-vc (mesh-pos *floor-mesh*) *floor-uv* *floor-vc*
                   (mesh-idx *floor-mesh*) (mat-identity) mvp 0)
    (gpu3d-mesh-vc (mesh-pos *ceil-mesh*) *ceil-uv* *ceil-vc*
                   (mesh-idx *ceil-mesh*) (mat-identity) mvp 0)
    ;; стены
    (gpu3d-mesh-vc (car *walls*) *wall-uv* (cadr *walls*) (caddr *walls*)
                   (mat-identity) mvp 0)
    ;; спрайты-билборды
    (gpu3d-alpha-blend 0.92)
    ;; выход-портал
    (gpu3d-batch-tex (mesh-pos *bb-mesh*) *bb-uv* (mesh-idx *bb-mesh*)
                     *exit-tex*
                     (list (car *exit*) 0.0 (cadr *exit*) 1.5 2.5 1.0 by 0 0
                           1 1 1)
                     mvp)
    ;; пикапы
    (define (item-insts it acc)
      (if (null? it)
          acc
          (let* ((i (car it))
                 (bob (+ 0.12 (* 0.06 (sin (+ (* *t* 3.2) (car i)))))))
            (item-insts (cdr it)
                        (append acc
                                (list (car i) bob (cadr i) 0.55 0.55 1.0 by 0 0
                                      1 1 1))))))
    (gpu3d-batch-tex (mesh-pos *bb-mesh*) *bb-uv* (mesh-idx *bb-mesh*)
                     *med-tex*
                     (item-insts
                      (let ((m '()))
                        (define (f it) (if (null? it) (reverse m) (begin (if (eq? (caddr (car it)) 'medkit) (set! m (cons (car it) m)) #f) (f (cdr it)))))
                        (f *items*))
                      '())
                     mvp)
    (gpu3d-batch-tex (mesh-pos *bb-mesh*) *bb-uv* (mesh-idx *bb-mesh*)
                     *ammo-tex*
                     (item-insts
                      (let ((m '()))
                        (define (f it) (if (null? it) (reverse m) (begin (if (eq? (caddr (car it)) 'ammo) (set! m (cons (car it) m)) #f) (f (cdr it)))))
                        (f *items*))
                      '())
                     mvp)
    ;; демоны: живые и трупы
    (define (einsts es acc)
      (if (null? es)
          acc
          (let* ((e (car es))
                 (hp (caddr e)) (hit (nth e 6))
                 (ph (nth e 4))
                 (wob (* 0.06 (sin (+ (* *t* 6.0) (nth e 5)))))
                 (hot (if (< (- *t* hit) 0.16) 1 0)))
            (if (<= hp 0)
                (einsts (cdr es)
                        (append acc (list (car e) 0.0 (cadr e) 1.5 0.35 1.0 by 0 0
                                          1 1 1)))
                (einsts (cdr es)
                        (append acc
                                (list (car e) wob (cadr e) 1.1 1.55 1.0 by 0 0
                                      (+ 1.0 (* 0.9 hot)) 0.75 (* 0.8 hot))))))))
    (gpu3d-batch-tex (mesh-pos *bb-mesh*) *bb-uv* (mesh-idx *bb-mesh*)
                     *gore-tex* (einsts *enemies* '()) mvp)
    (gpu3d-batch-tex (mesh-pos *bb-mesh*) *bb-uv* (mesh-idx *bb-mesh*)
                     *imp-tex* (einsts *enemies* '()) mvp)
    ;; вспышка выстрела
    (if (< (- *t* *muzzle*) 0.07)
        (gpu3d-batch-tex (mesh-pos *bb-mesh*) *bb-uv* (mesh-idx *bb-mesh*)
                         *flash-tex*
                         (list (+ *px* (* 0.9 (cos *pa*))) 0.95
                               (+ *pz* (* 0.9 (sin *pa*)))
                               0.7 0.7 1.0 by 0 0  1 1 1)
                         mvp)
        #f)
    (gpu3d-alpha-blend 1.0)
    (gpu3d-flush-alpha)
    (gpu3d-post 1.05 0.5 0.25 0.045)
    mvp))

;; ——— HUD ———
(define (draw-hud)
  (if (> *hurt-flash* 0.02)
      (gpu-fill-rect 0 0 320 240
                     (* 0.28 *hurt-flash*) 0.02 0.02)
      #f)
  (draw-text (string-append "ЗДОРОВЬЕ " (number->string (floor *hp*))
                            "  ПАТРОНЫ " (number->string *ammo*)
                            "  УРОВЕНЬ " (number->string *level*)
                            "  ОЧКИ " (number->string *score*))
             5 5 1 (list 0.95 0.9 0.75))
  (if (> (string-length *msg*) 0)
      (draw-text *msg* 5 13 1 (list 1 0.85 0.35))
      #f)
  ;; прицел
  (gpu-fill-rect 157 115 6 2 1 0.3 0.25)
  (gpu-fill-rect 159 113 2 6 1 0.3 0.25)
  (if (eq? *state* 'dead)
      (begin
        (gpu-fill-rect 0 0 320 240 0.35 0.02 0.02)
        (draw-text "ТЫ ПОГИБ" 116 100 2 (list 1 0.85 0.8))
        (draw-text (string-append "ОЧКИ " (number->string *score*)
                                  "   R — ЗАНОВО") 88 120 1
                   (list 0.9 0.9 0.9)))
      #f))

(define (draw)
  (draw-scene)
  (draw-hud)
  (gpu-present))

;; ——— апдейт ———
(define (update dt)
  (poll-input)
  (set! *t* (+ *t* dt))
  (set! *frame* (+ *frame* 1))
  (set! *hurt-flash* (max 0.0 (- *hurt-flash* (* 3.0 dt))))
  (draw-text (string-append "P=" (number->string (floor *px*)) ","
                            (number->string (floor *pz*)) " A="
                            (number->string (floor *pa*)) " W="
                            (number->string (f32ref *grid* (g-ix (floor *px*) (floor *pz*)))))
             5 29 1 (list 0.6 0.8 0.9))
  (if (> (string-length *msg*) 0)
      (if (> *t* (+ *msg-t* 2.5)) (set! *msg* "") #f)
      #f)
  (if (eq? *state* 'dead)
      (if (key-pressed? "r")
          (begin
            (set! *state* 'play)
            (set! *hp* 100) (set! *ammo* 30)
            (set! *level* 1) (set! *score* 0) (set! *kills* 0)
            (spawn-level))
          #f)
      (begin
        (if (or (key-pressed? " ") (key-pressed? "space")) (fire!) #f)

        (let* ((turn 2.1) (walk 3.6))
          (if (key-down? "a") (set! *pa* (+ *pa* (* turn dt))) #f)
          (if (key-down? "d") (set! *pa* (- *pa* (* turn dt))) #f)
          (let* ((dx (cos *pa*)) (dz (sin *pa*))
                 (sp (* walk dt))
                 (mv 0))
            (if (key-down? "w")
                (let ((np (move-actor! (list *px* *pz*) (* dx sp) (* dz sp))))
                  (set! *px* (car np)) (set! *pz* (cadr np)))
                #f)
            (if (key-down? "s")
                (let ((np (move-actor! (list *px* *pz*) (- (* dx sp)) (- (* dz sp)))))
                  (set! *px* (car np)) (set! *pz* (cadr np)))
                #f)
            (if (key-down? "q")
                (let ((np (move-actor! (list *px* *pz*) (* dz sp) (- (* dx sp)))))
                  (set! *px* (car np)) (set! *pz* (cadr np)))
                #f)
            (if (key-down? "e")
                (let ((np (move-actor! (list *px* *pz*) (- (* dz sp)) (* dx sp))))
                  (set! *px* (car np)) (set! *pz* (cadr np)))
                #f)))
        (update-enemies! dt)
        (update-items!)
        (check-exit!)
        (if (<= *hp* 0)
            (begin (set! *state* 'dead) (play-sfx 'hurt))
            #f)))
  (if (>= *frame* (+ *fps-last* 120))
      (begin
        (set! *fps-val* (/ 120.0 (max 0.001 (- (time-now) *fps-clock*))))
        (set! *fps-clock* (time-now))
        (set! *fps-last* *frame*))
      #f)
  #t)

(define *fps-clock* (time-now))
(spawn-level)
(rebuild-meshes!)
(print (string-append "WALL-LEN " (number->string (f32vec-length (car *walls*)))))
(print "RUINER TAIL")
(print "TAIL-OK")
(print (string-append "RUINER:urov " (number->string *level*) " demonov " (number->string (length *enemies*))))
