;; shtolnya.lisp — «ШТОЛЬНЯ» (этап 25): хоррор от первого лица, весь геймплей на Lisp.
;; Фонарь с батареей (gpu-light-spot), лабиринт-штольня (батч стен), монстр
;; (телепорт-сталкер: замирает, когда на него смотрят), 8 записок, звук на
;; чиптюн-синтезе, сердцебиение/статика/скример.
;; Нативы этапа 25: gpu-fill-rect-a (тьма/вспышки/статика), gpu-light-spot
;; (фонарь, блики записок/ламп/маски).
;; Запуск: rlisp-engine --script assets/lisp/main_shtolnya.lisp --width 400 --height 300

;; ——— параметры мира ———
(define *cell* 4.0)
(define *wall-h* 3.4)
(define *eye-h* 1.62)
(define *maze* (list
  "#############"
  "#S..#...#...#"
  "#.#.#.#.#.#.#"
  "#.#...#.....#"
  "#.#####.###.#"
  "#.....#.#...#"
  "#.###.#.#.###"
  "#.#...#.#...#"
  "#.#.###.###.#"
  "#.#.#.....#.#"
  "#...#.###.#.#"
  "###.#.#.#.#.#"
  "#.....#...#P#"
  "#############"))

(define *screen-w* 400)
(define *screen-h* 300)

;; ——— состояние игрока ———
(define *state* "intro")
(define *game-t* 0.0)
(define *px* 6.0)
(define *pz* 6.0)
(define *yaw* 2.35)
(define *bob* 0.0)
(define *shake* 0.0)
(define *torch* #t)
(define *batt* 100.0)
(define *spare* 2)
(define *pages* 0)
(define *total-pages* 8)
(define *near-page* #f)
(define *flash* 0.0)
(define *red* 0.0)
(define *static* 0.0)
(define *heart-t* 0.0)
(define *step-t* 0.0)
(define *drone-t* 0.0)
(define *end-t* 0.0)

;; ——— монстр ———
(define *mx* 44.0)
(define *mz* 8.0)
(define *m-mode* "idle")
(define *m-t* 0.0)
(define *m-see* 0.0)

;; ——— подбираемое: мутация через таблицы (list-set! в stdlib нет) ———
(define *page-taken* (make-table))
(define *battery-taken* (make-table))

;; записки: (cx cz ndx ndz) — мир и нормаль стены
(define *page-spots* (list
  (list 46.0 40.0 0.0 1.0)
  (list 46.0 36.0 0.0 1.0)
  (list 30.0 46.0 0.0 -1.0)
  (list 46.0 32.0 0.0 1.0)
  (list 30.0 50.0 1.0 0.0)
  (list 46.0 28.0 0.0 1.0)
  (list 34.0 50.0 1.0 0.0)
  (list 42.0 28.0 0.0 -1.0)))

(define *battery-spots* (list (list 6.0 10.0) (list 34.0 14.0) (list 14.0 38.0) (list 42.0 22.0)))

(define *lamps* (list
  (list 10.0 10.0 0.0)
  (list 18.0 22.0 1.7)
  (list 30.0 30.0 3.1)
  (list 42.0 42.0 0.9)
  (list 26.0 46.0 2.2)))

;; ——— матрицы: умножение 4×4 (column-vector, трансляция в 4-м столбце) ———
(define (mat-mul a b)
  (let ((out (f32-from-list (list 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0))))
    (define (go-r r)
      (if (< r 4)
          (begin
            (define (go-c c)
              (if (< c 4)
                  (begin
                    (f32set! out (+ (* r 4) c)
                            (+ (* (f32ref a (+ (* r 4) 0)) (f32ref b (+ 0 c)))
                               (* (f32ref a (+ (* r 4) 1)) (f32ref b (+ 4 c)))
                               (* (f32ref a (+ (* r 4) 2)) (f32ref b (+ 8 c)))
                               (* (f32ref a (+ (* r 4) 3)) (f32ref b (+ 12 c)))))
                    (go-c (+ c 1)))
                  #f))
            (go-c 0)
            (go-r (+ r 1)))
          #f))
    (go-r 0)
    out))

;; клетка мира → символ карты
(define (tile-at wx wz)
  (let* ((cx (floor (/ wx *cell*)))
         (cz (floor (/ wz *cell*)))
         (czc (max 0 (min 13 cz)))
         (cxc (max 0 (min 12 cx))))
    (string-at (nth *maze* czc) cxc)))

(define (wall? wx wz) (string=? (tile-at wx wz) "#"))

;; выталкивание из одной стены-клетки (AABB + радиус)
(define (push-out px pz cx cz r)
  (if (not (wall? (+ (* cx *cell*) 0.01) (+ (* cz *cell*) 0.01)))
      (list px pz)
      (let* ((x0 (* cx *cell*)) (z0 (* cz *cell*))
             (x1 (+ x0 *cell*)) (z1 (+ z0 *cell*))
             (nx (max x0 (min x1 px)))
             (nz (max z0 (min z1 pz)))
             (dx (- px nx)) (dz (- pz nz))
             (d2 (+ (* dx dx) (* dz dz))))
        (if (>= d2 (* r r))
            (list px pz)
            (if (< d2 0.0001)
                (list (+ x0 (/ *cell* 2.0)) (+ z0 (/ *cell* 2.0)))
                (let* ((d (sqrt d2)) (k (/ (- r d) d)))
                  (list (+ px (* dx k)) (+ pz (* dz k)))))))))

;; коллизия: 4 клетки вокруг игрока
(define (collide! px pz r)
  (let* ((bx (floor (/ px *cell*)))
         (bz (floor (/ pz *cell*)))
         (a (push-out px pz bx bz r))
         (b (push-out (car a) (cadr a) (+ bx 1) bz r))
         (c (push-out (car b) (cadr b) bx (+ bz 1) r))
         (d (push-out (car c) (cadr c) (+ bx 1) (+ bz 1) r)))
    d))

(define (free-cell? cx cz)
  (and (>= cx 0) (<= cx 12) (>= cz 0) (<= cz 13)
       (not (string=? (string-at (nth *maze* cz) cx) "#"))))

;; телепорт монстра: ТОЛЬКО на приближение — новая клетка в кольце 6..14 м
;; от игрока и строже прежней дистанции (иначе баланс «ползёт-откидывается»
;; держит его на месте)
(define (monster-teleport!)
  (let* ((cx0 (floor (/ *mx* *cell*)))
         (cz0 (floor (/ *mz* *cell*)))
         (dx0 (- *mx* *px*)) (dz0 (- *mz* *pz*))
         (d0 (sqrt (+ (* dx0 dx0) (* dz0 dz0)))))
    (define (try-one k)
      (let* ((cx (mod (+ (* k 7) cx0) 13))
             (cz (mod (+ (* (+ k 3) 11) cz0) 14)))
        (if (not (free-cell? cx cz))
            #f
            (let* ((wx (+ (* cx *cell*) 2.0))
                   (wz (+ (* cz *cell*) 2.0))
                   (dx (- wx *px*)) (dz (- wz *pz*))
                   (d (sqrt (+ (* dx dx) (* dz dz)))))
              (if (and (> d 5.5) (< d 14.0) (< d (- d0 1.0)))
                  (begin (set! *mx* wx) (set! *mz* wz) #t)
                  #f)))))
    (define (go k)
      (if (>= k 64)
          #f
          (if (try-one (+ k (floor (/ (random 9) 1.0)))) #t (go (+ k 1)))))
    (go 0)))

;; ——— меши ———
(define *wall-mesh* (mesh-cube 0.5))
(define *pole-mesh* (mesh-cube 0.5))
(define *page-mesh* (mesh-make (list -0.28 -0.38 0  0.28 -0.38 0  0.28 0.38 0  -0.28 0.38 0)
                               (list 0 1 2  0 2 3  2 1 0  3 2 0)))
(define *monster-mesh* (mesh-make (list -0.34 0 0  0.34 0 0  0.34 2.15 0  -0.34 2.15 0
                                        -0.30 0 0  0.30 0 0  0.30 2.05 0  -0.30 2.05 0)
                                  (list 0 1 2  0 2 3  2 1 0  3 2 0
                                        4 5 6  4 6 7)))
(define *face-mesh* (mesh-make (list -0.17 1.78 0  0.17 1.78 0  0.17 2.08 0  -0.17 2.08 0)
                               (list 0 1 2  0 2 3  2 1 0  3 2 0)))
(define *lamp-mesh* (mesh-cube 0.5))

;; ——— звук: всё на чиптюн-синтезе ———
(define (sfx-init!)
  (defsound-synth 'drone "sine" 44 4.0 2.0 0.2 0.9 0.6)
  (defsound-synth 'thump "sine" 52 0.11 0.005 0.02 0.2 0.05)
  (defsound-synth 'sting "saw" 320 0.55 0.004 0.12 0.5 0.3 -0.85)
  (defsound-synth 'page "square" 660 0.14 0.005 0.03 0.3 0.08)
  (defsound-synth 'step "noise" 240 0.07 0.004 0.02 0.3 0.05)
  (defsound-synth 'torch "square" 1200 0.05 0.002 0.01 0.2 0.03)
  (defsound-synth 'battery "triangle" 880 0.18 0.004 0.04 0.4 0.1 0.3)
  (defsound-synth 'win "triangle" 520 0.7 0.01 0.1 0.6 0.4 0.25))

;; ——— проекция мировой точки на экран: (sx sy visible)
(define (project-point mvp wx wy wz)
  (let* ((cx (+ (* (f32ref mvp 0) wx) (* (f32ref mvp 1) wy)
                (* (f32ref mvp 2) wz) (f32ref mvp 3)))
         (cy (+ (* (f32ref mvp 4) wx) (* (f32ref mvp 5) wy)
                (* (f32ref mvp 6) wz) (f32ref mvp 7)))
         (cw (+ (* (f32ref mvp 12) wx) (* (f32ref mvp 13) wy)
                (* (f32ref mvp 14) wz) (f32ref mvp 15))))
    (if (<= cw 0.05)
        (list 0 0 #f)
        (let* ((sx (+ 0.5 (* 0.5 (/ cx cw))))
               (sy (- 0.5 (* 0.5 (/ cy cw)))))
          (list (* sx *screen-w*) (* sy *screen-h*)
                (and (>= sx -0.2) (<= sx 1.2) (>= sy -0.2) (<= sy 1.2)))))))

;; ——— обновление подбираемого ———
(define (update-pickups!)
  (set! *near-page* #f)
  (define (scan-pages i)
    (if (>= i (length *page-spots*))
        #f
        (let* ((spot (nth *page-spots* i))
               (taken (table-get *page-taken* i #f))
               (dx (- (car spot) *px*)) (dz (- (cadr spot) *pz*))
               (d (sqrt (+ (* dx dx) (* dz dz)))))
          (if (and (not taken) (< d 1.5))
              (set! *near-page* i)
              #f)
          (scan-pages (+ i 1)))))
  (scan-pages 0)
  (define (scan-batt i)
    (if (>= i (length *battery-spots*))
        #f
        (let* ((spot (nth *battery-spots* i))
               (taken (table-get *battery-taken* i #f))
               (dx (- (car spot) *px*)) (dz (- (cadr spot) *pz*))
               (d (sqrt (+ (* dx dx) (* dz dz)))))
          (if (and (not taken) (< d 1.3) (< *batt* 55.0))
              (begin (table-set! *battery-taken* i #t)
                     (set! *spare* (+ *spare* 1))
                     (play-sfx 'battery 0.8))
              #f)
          (scan-batt (+ i 1)))))
  (scan-batt 0))

;; ——— монстр: замер при взгляде, телепорт вне взгляда ———
(define (update-monster dt)
  (let* ((mdx (- *mx* *px*)) (mdz (- *mz* *pz*))
         (md (sqrt (+ (* mdx mdx) (* mdz mdz))))
         (dirx (/ mdx (max 0.01 md))) (dirz (/ mdz (max 0.01 md)))
         (lookx (sin *yaw*)) (lookz (- 0 (cos *yaw*)))
         (dot (+ (* lookx dirx) (* lookz dirz))))
    (if (and (> dot 0.86) (< md 18.0))
        (begin (set! *m-mode* "frozen")
               (set! *m-see* (+ *m-see* dt))
               (set! *static* (min 1.0 (+ 0.15 (* 0.35 *m-see*))))
               (if (< md 4.5) (set! *shake* 0.6) #f))
        (begin
          (set! *m-see* (max 0.0 (- *m-see* (* 2.0 dt))))
          (set! *static* (max 0.0 (- *static* (* 2.5 dt))))
          (set! *m-mode* "stalk")))
    ;; призрачное преследование: вне взгляда ползёт к игроку (1.1 м/с),
    ;; после 4 записок — быстрее; стены игнорирует (штольня помнит все ходы)
    (if (and (string=? *m-mode* "stalk") (> md 2.0))
        (let* ((sp (if (>= *pages* 4) 1.8 1.4))
               (nx (+ *mx* (* dirx sp dt)))
               (nz (+ *mz* (* dirz sp dt))))
          (set! *mx* nx)
          (set! *mz* nz))
        #f)
    ;; долгий взгляд бесит монстра: рывок К ИГРОКУ (до 1.2 м — ближе смерть)
    (if (and (string=? *m-mode* "frozen") (> *m-see* 2.2))
        (let* ((lx (- *mx* (* dirx (max 0.6 (- md 1.35)))))
               (lz (- *mz* (* dirz (max 0.6 (- md 1.35))))))
          (set! *mx* lx)
          (set! *mz* lz)
          (set! *m-see* 0.6)
          (set! *shake* 0.8)
          (play-sfx 'sting 0.8))
        #f)
    (set! *m-t* (+ *m-t* dt))
    (let ((period (if (string=? *m-mode* "frozen") 3.4 (if (>= *pages* 4) 1.9 2.7))))
      (if (> *m-t* period)
          (begin (set! *m-t* 0.0)
                 (if (string=? *m-mode* "stalk")
                     (monster-teleport!)
                     #f))
          #f))
    ;; контакт — скример и смерть
    (if (< md 1.7)
        (begin (set! *state* "dead")
               (set! *end-t* *game-t*)
               (set! *red* 1.0)
               (set! *flash* 1.0)
               (play-sfx 'sting 1.0))
        #f)
    md))

;; ——— update ———
(define (update dt)
  (poll-input)
  (set! *game-t* (+ *game-t* dt))
  (set! *flash* (max 0.0 (- *flash* (* 2.2 dt))))
  (set! *shake* (max 0.0 (- *shake* (* 3.0 dt))))
  (set! *red* (if (string=? *state* "dead")
                 (max 0.35 (- *red* (* 0.25 dt)))
                 (max 0.0 (- *red* (* 0.8 dt)))))
  ;; интро: любой ход — старт
  (if (and (string=? *state* "intro")
           (or (key-pressed? "w") (key-pressed? " ")))
      (begin (set! *state* "play")
             (set! *game-t* 0.0)
             (play-sfx 'torch 0.6))
      #f)
  ;; смерть/победа: R — рестарт
  (if (and (or (string=? *state* "dead") (string=? *state* "win")) (key-pressed? "r"))
      (begin (set! *state* "play")
             (set! *px* 6.0) (set! *pz* 6.0) (set! *yaw* 2.35)
             (set! *batt* 100.0) (set! *spare* 2) (set! *pages* 0)
             (set! *page-taken* (make-table))
             (set! *battery-taken* (make-table))
             (set! *mx* 44.0) (set! *mz* 8.0) (set! *m-mode* "idle")
             (set! *m-t* 0.0) (set! *m-see* 0.0) (set! *static* 0.0)
             (set! *game-t* 0.0))
      #f)
  (if (string=? *state* "play")
      (let ((mv 0.0))
        (if (key-down? "a") (set! *yaw* (+ *yaw* (* 2.2 dt))) #f)
        (if (key-down? "d") (set! *yaw* (- *yaw* (* 2.2 dt))) #f)
        (if (key-down? "w") (set! mv 2.4) #f)
        (if (key-down? "s") (set! mv -1.7) #f)
        (if (> (abs mv) 0.01)
            (let* ((nx (+ *px* (* (sin *yaw*) mv dt)))
                   (nz (+ *pz* (* (- 0 (cos *yaw*)) mv dt)))
                   (col (collide! nx nz 0.42)))
              (set! *px* (car col))
              (set! *pz* (cadr col))
              (set! *bob* (+ *bob* (* dt 9.0 (if (> mv 0) 1.0 -1.0))))
              (set! *step-t* (+ *step-t* dt))
              (if (> *step-t* 0.42)
                  (begin (set! *step-t* 0.0) (play-sfx 'step 0.25 0.9))
                  #f))
            (set! *bob* (- *bob* (* dt 2.0 *bob*))))
        ;; фонарь
        (if (key-pressed? "f")
            (begin (set! *torch* (not *torch*)) (play-sfx 'torch 0.5))
            #f)
        (if (and *torch* (> *batt* 0.0))
            (set! *batt* (max 0.0 (- *batt* (* 1.35 dt))))
            #f)
        (if (and (key-pressed? "e") (> *spare* 0) (< *batt* 30.0))
            (begin (set! *spare* (- *spare* 1))
                   (set! *batt* 100.0)
                   (play-sfx 'battery 0.7))
            #f)
        ;; подбор
        (update-pickups!)
        (if (and *near-page* (key-pressed? " "))
            (begin (table-set! *page-taken* *near-page* #t)
                   (set! *pages* (+ *pages* 1))
                   (set! *flash* 0.5)
                   (play-sfx 'page 0.8))
            #f)
        ;; монстр + сердцебиение + гул
        (let ((md (update-monster dt)))
          (set! *heart-t* (+ *heart-t* dt))
          (let ((period (max 0.42 (min 1.6 (/ md 9.0)))))
            (if (and (> *heart-t* period) (< md 20.0))
                (begin (set! *heart-t* 0.0)
                       (play-sfx 'thump (max 0.15 (min 1.0 (- 1.15 (/ md 22.0))))))
                #f))
          (set! *drone-t* (+ *drone-t* dt))
          (if (> *drone-t* 3.8)
              (begin (set! *drone-t* 0.0) (play-sfx 'drone 0.22))
              #f)
          ;; победа: 8/8 и клетка выхода P(11,12)
          (if (and (>= *pages* *total-pages*) (> *px* 42.0) (> *pz* 46.0))
              (begin (set! *state* "win") (set! *end-t* *game-t*))
              #f)))
      #f))

;; ——— инстансы стен: клетка '#' → куб 4×4×4 ———
;; draw-batch ждёт ПЛОСКИЙ список чисел (как в phys_poly: *trunk-insts*)
(define *wall-out* '())
(define (wall-push! x y z s yaw r g b)
  (set! *wall-out* (append *wall-out* (list x y z s s s yaw 0.0 0.0 r g b))))

(define (build-wall-instances)
  (set! *wall-out* '())
  (define (go-z z)
    (if (>= z 14)
        #f
        (begin
          (define (go-x x)
            (if (>= x 13)
                #f
                (let ((row (nth *maze* z)))
                  (if (string=? (string-at row x) "#")
                      (begin
                        (wall-push! (+ (* x *cell*) 2.0) 2.0 (+ (* z *cell*) 2.0)
                                    4.0 0.0 0.34 0.32 0.30)
                        (go-x (+ x 1)))
                      (go-x (+ x 1))))))
          (go-x 0)
          (go-z (+ z 1)))))
  (go-z 0)
  *wall-out*)

(define *wall-instances* #f)

;; безопасный символ строки (край карты)
(define (str-at row i)
  (if (and (>= i 0) (< i (string-length row)))
      (string-at row i)
      "."))

;; сетка пола/потолка: вершины 0..52 × 0..56 с шагом клетки, y = yv
(define (build-grid-mesh yv)
  (let ((pos '()) (idx '()) (vi 0))
    (define (go-z z)
      (if (> z 14)
          #f
          (begin
            (define (go-x x)
              (if (> x 13)
                  #f
                  (begin
                    (set! pos (append pos (list (* x *cell*) yv (* z *cell*))))
                    (go-x (+ x 1)))))
            (go-x 0)
            (go-z (+ z 1)))))
    (go-z 0)
    ;; индексы: клетка (x,z) → вершины в ряду z: (14 строк × 14 столбцов)
    (define (go-iz z)
      (if (>= z 14)
          #f
          (begin
            (define (go-ix x)
              (if (>= x 13)
                  #f
                  (let* ((a (+ (* z 14) x))
                         (b (+ a 1))
                         (c (+ a 14))
                         (d (+ c 1)))
                    (set! idx (append idx (list a c b  b c d)))
                    (go-ix (+ x 1)))))
            (go-ix 0)
            (go-iz (+ z 1)))))
    (go-iz 0)
    (mesh-make pos idx)))

(define *floor-mesh* #f)
(define *ceil-mesh* #f)

(define (draw-world mvp t)
  ;; пол и потолок — СЕТКА 13×14 (гигантские треугольники дают афинные
  ;; артефакты интерполяции — полосы на скрин-спейсе)
  (if (not *floor-mesh*)
      (begin
        (set! *floor-mesh* (build-grid-mesh 0.0))
        (set! *ceil-mesh* (build-grid-mesh 3.4)))
      #f)
  (draw-mesh *floor-mesh* (mat-identity) mvp (list 0.22 0.20 0.18))
  (draw-mesh *ceil-mesh* (mat-identity) mvp (list 0.16 0.16 0.18))
  ;; стены одним батчем
  (if *wall-instances* (draw-batch *wall-mesh* *wall-instances* mvp) #f)
  ;; крепи: пары столбов в свободных клетках (x+z) кратно 3
  (define poles '())
  (define (pole-push! x y z s yaw r g b)
    (set! poles (append poles (list x y z s s s yaw 0.0 0.0 r g b))))
  (define (go-z z)
    (if (>= z 13)
        (draw-batch *pole-mesh* poles mvp)
        (begin
          (define (go-x x)
            (if (>= x 12)
                #f
                (let* ((row (nth *maze* z))
                       (ch (str-at row x)))
                  (if (and (string=? ch ".") (= (mod (+ x z) 3) 0))
                      (let* ((wx (+ (* x *cell*) 2.0)) (wz (+ (* z *cell*) 2.0))
                             (vert (and (string=? (str-at row (- x 1)) "#")
                                        (string=? (str-at row (+ x 1)) "#")))
                             (ox (if vert 1.5 0.0)) (oz (if vert 0.0 1.5)))
                        (pole-push! (+ wx ox) 0.85 wz 3.4 0.0 0.55 0.40 0.26)
                        (pole-push! (- wx ox) 0.85 wz 3.4 0.0 0.55 0.40 0.26)
                        (go-x (+ x 1)))
                      (go-x (+ x 1))))))
          (go-x 1)
          (go-z (+ z 1)))))
  (go-z 1)
  ;; лампы-плафоны (мерцают)
  (define (go-l i)
    (if (>= i (length *lamps*))
        #f
        (let* ((lamp (nth *lamps* i))
               (ph (caddr lamp))
               (blink (if (< (sin (+ (* t 13.0) (* ph 7.0))) -0.82) 0.12 1.0)))
          (draw-mesh *lamp-mesh* (mat-identity) mvp
                     (list (* 0.9 blink) (* 0.8 blink) (* 0.5 blink)))
          (go-l (+ i 1)))))
  (go-l 0)
  ;; записки на стенах
  (define (go-p i)
    (if (>= i (length *page-spots*))
        #f
        (let* ((spot (nth *page-spots* i))
               (taken (table-get *page-taken* i #f)))
          (if (not taken)
              (let* ((cx (car spot)) (cz (cadr spot))
                     (dx (caddr spot)) (dz (cadddr spot))
                     (px (+ cx (* dx 1.72))) (pz (+ cz (* dz 1.72)))
                     (m (mat-mul (mat-translate px 1.55 pz)
                                 (mat-rotate-y (if (> (abs dx) 0.1) 1.5708 0.0)))))
                (draw-mesh *page-mesh* m mvp (list 0.95 0.93 0.85)))
              #f)
          (go-p (+ i 1)))))
  (go-p 0)
  ;; батарейки на полу
  (define (go-b i)
    (if (>= i (length *battery-spots*))
        #f
        (let* ((spot (nth *battery-spots* i))
               (taken (table-get *battery-taken* i #f)))
          (if (not taken)
              (let ((m (mat-mul (mat-translate (car spot) 0.12 (cadr spot))
                                (mat-rotate-y (* t 1.3)))))
                (draw-mesh *pole-mesh* m mvp (list 0.3 0.75 0.3)))
              #f)
          (go-b (+ i 1)))))
  (go-b 0)
  ;; монстр: тёмный силуэт + белая маска (лицом к игроку)
  (if (string=? *state* "dead")
      #f
      (let* ((m (mat-mul (mat-translate *mx* 0.0 *mz*)
                         (mat-rotate-y (+ 3.1416 (atan2 (- *px* *mx*) (- *pz* *mz*)))))))
        (draw-mesh *monster-mesh* m mvp (list 0.05 0.05 0.06))
        (draw-mesh *face-mesh* m mvp (list 0.92 0.92 0.88)))))

;; ——— блики: светящиеся объекты после затемнения (project-point) ———
(define (draw-glow mvp t)
  ;; записки
  (define (go-p i)
    (if (>= i (length *page-spots*))
        #f
        (let* ((spot (nth *page-spots* i))
               (taken (table-get *page-taken* i #f)))
          (if (not taken)
              (let* ((pr (project-point mvp (car spot) 1.55 (cadr spot))))
                (if (caddr pr)
                    (let* ((sx (car pr)) (sy (cadr pr))
                           (dx (- (car spot) *px*)) (dz (- (cadr spot) *pz*))
                           (md (sqrt (+ (* dx dx) (* dz dz))))
                           (k (max 0.0 (min 0.85 (- 1.0 (/ md 30.0))))))
                      (gpu-light-spot sx sy 13.0 (* 0.9 k) 1.0 0.97 0.85))
                    #f))
              #f)
          (go-p (+ i 1)))))
  (go-p 0)
  ;; лампы
  (define (go-l i)
    (if (>= i (length *lamps*))
        #f
        (let* ((lamp (nth *lamps* i))
               (pr (project-point mvp (car lamp) 3.2 (cadr lamp))))
          (if (caddr pr)
              (let* ((sx (car pr)) (sy (cadr pr))
                     (blink (if (< (sin (+ (* t 13.0) (* (caddr lamp) 7.0))) -0.82) 0.1 1.0))
                     (dx (- (car lamp) *px*)) (dz (- (cadr lamp) *pz*))
                     (md (sqrt (+ (* dx dx) (* dz dz))))
                     (k (* blink (max 0.0 (min 0.9 (- 1.0 (/ md 26.0)))))))
                (gpu-light-spot sx sy 22.0 (* 1.1 k) 1.0 0.85 0.55))
              #f))
        (go-l (+ i 1))))
  (go-l 0)
  ;; маска монстра — светится в темноте
  (if (string=? *state* "dead")
      #f
      (let* ((pr (project-point mvp *mx* 1.93 *mz*)))
        (if (caddr pr)
            (let* ((sx (car pr)) (sy (cadr pr))
                   (dx (- *mx* *px*)) (dz (- *mz* *pz*))
                   (md (sqrt (+ (* dx dx) (* dz dz))))
                   (k (max 0.0 (min 1.0 (- 1.05 (/ md 22.0))))))
              (gpu-light-spot sx sy (+ 9.0 (* 7.0 k)) (* 1.8 k) 1.0 1.0 0.95))
            #f))))

;; полосы помех при взгляде на монстра
(define (draw-static-bands)
  (define (bands k)
    (if (>= k 14)
        #f
        (let* ((y (mod (* (floor (* *game-t* 60.0)) (+ k 7)) *screen-h*))
               (h (+ 2 (mod k 4))))
          (gpu-fill-rect-a 0 y *screen-w* h 0.75 0.75 0.78 (* 0.22 *static*))
          (bands (+ k 1)))))
  (bands 0))

;; ——— draw ———
(define (draw)
  (let ((wh (window-size)))
    (set! *screen-w* (car wh))
    (set! *screen-h* (cadr wh)))
  (if (not *wall-instances*) (set! *wall-instances* (build-wall-instances)) #f)
  (let* ((t *game-t*)
         (eye-y (+ *eye-h* (* 0.045 (sin *bob*))))
         (shx (* 0.03 *shake* (sin (* t 43.0))))
         (shy (* 0.03 *shake* (cos (* t 37.0))))
         (eye (list (+ *px* shx) (+ eye-y shy) *pz*))
         (dead-drop (if (string=? *state* "dead")
                        (* -1.1 (min 1.0 (- *game-t* *end-t* 0.2)))
                        0.0))
         (eye2 (list (car eye) (+ (cadr eye) dead-drop) (caddr eye)))
         (target (list (+ *px* (* 10 (sin *yaw*)))
                       (+ (cadr eye2) -0.6)
                       (+ *pz* (* 10 (- 0 (cos *yaw*))))))
         (mvp (camera-mvp eye2 target 0.9)))
    (gpu3d-clear 0.005 0.006 0.010)
    (gpu3d-fog 0.01 0.011 0.016 0.05)
    (gpu3d-view (car eye2) (cadr eye2) (caddr eye2))
    (draw-world mvp t)
    ;; пост: виньетка дышит с опасностью, зерно растёт от статики
    (let* ((dx (- *mx* *px*)) (dz (- *mz* *pz*))
           (md (sqrt (+ (* dx dx) (* dz dz))))
           (danger (max 0.0 (min 1.0 (- 1.0 (/ md 16.0))))))
      (gpu3d-post 0.9
                  0.0
                  (+ 0.5 (* 0.16 danger) (* 0.10 *static*))
                  (+ 0.10 (* 0.45 *static*))))
    ;; тьма
    (gpu-fill-rect-a 0 0 *screen-w* *screen-h* 0.0 0.0 0.0 0.42)
    ;; фонарь: центр экрана, мерцание при разряде
    (let* ((cx (* 0.5 *screen-w*))
           (cy (* 0.47 *screen-h*))
           (flick (if (< *batt* 25.0)
                      (if (< (sin (+ (* t 31.0) (* t 7.7))) -0.4) 0.25 1.0)
                      1.0))
           (on (if (and *torch* (> *batt* 0.0) (not (string=? *state* "dead"))) 1.0 0.0))
           (rad (* (min *screen-w* *screen-h*) 0.42))
           (inten (* 0.75 on flick)))
      (gpu-light-spot cx cy rad inten 1.0 0.96 0.86)
      (gpu-light-spot cx cy (* rad 0.5) (* 0.55 on flick) 1.0 0.97 0.9)
      (gpu-light-spot cx cy (* rad 0.45) (* 0.8 on flick) 1.0 0.99 0.93))
    (draw-glow mvp t)
    ;; вспышка скримера
    (if (> *flash* 0.01)
        (gpu-fill-rect-a 0 0 *screen-w* *screen-h* 1.0 1.0 1.0 (* 0.85 *flash*))
        #f)
    ;; красная гибель
    (if (> *red* 0.01)
        (gpu-fill-rect-a 0 0 *screen-w* *screen-h* 0.5 0.0 0.0 (* 0.55 *red*))
        #f)
    ;; статика при взгляде на монстра
    (if (> *static* 0.42) (draw-static-bands) #f)
    (draw-hud)
    (gpu-present)))

;; ——— HUD ———
(define (draw-hud)
  (let ((w *screen-w*) (h *screen-h*))
    (if (string=? *state* "intro")
        (begin
          (draw-rect 0 0 w h (list 0.01 0.01 0.02))
          (draw-text "SHTOLNYA" (- (/ w 2) 44) 46 3 (list 0.85 0.82 0.75))
          (draw-text "8 ZAPISEK. FONAR. ON ZHDET." (- (/ w 2) 92) 96 1 (list 0.55 0.5 0.45))
          (draw-text "W/S HODBA  A/D POBOROT" (- (/ w 2) 74) 130 1 (list 0.4 0.42 0.4))
          (draw-text "F FONAR  E BATAREYA  SPACE VZAT" (- (/ w 2) 92) 146 1 (list 0.4 0.42 0.4))
          (draw-text "ZHEMI SPACE - NACHAT" (- (/ w 2) 66) 200 1 (list 0.8 0.75 0.6)))
        (begin
          (if (string=? *state* "dead")
              #f
              (draw-text (string-append "ZAPISKI " (number->string *pages*) "/8")
                         8 8 1 (list 0.8 0.78 0.7)))
          (if (string=? *state* "dead")
              #f
              (draw-rect 8 22 84 6 (list 0.08 0.08 0.08)))
          (let ((bw (* 80 (/ (max 0.0 *batt*) 100.0))))
            (draw-rect 10 24 bw 2
                       (if (< *batt* 25.0)
                           (list 0.9 0.25 0.15)
                           (list 0.85 0.75 0.35))))
          (draw-text (number->string *spare*) 100 22 1 (list 0.5 0.55 0.45))
          (draw-text (string-append "ST:" (number->string *static*) " T:" (number->string (floor *game-t*)))
                     150 22 1 (list 0.6 0.6 0.7))
          (if *near-page*
              (draw-text "SPACE - VZAT ZAPISKU" (- (/ w 2) 62) (- h 56) 1 (list 0.9 0.88 0.8))
              #f)
          (if (>= *pages* *total-pages*)
              (draw-text "VYHOD OTKRYT - IDI K VYHODU" (- (/ w 2) 96) 20 1 (list 0.7 0.9 0.6))
              #f)
          (if (string=? *state* "dead")
              (begin
                (draw-text "ON NASHEL TEBYA" (- (/ w 2) 66) 120 2 (list 0.9 0.2 0.15))
                (draw-text "R - ESHCHE RAZ" (- (/ w 2) 46) 160 1 (list 0.6 0.4 0.35)))
              #f)
          (if (string=? *state* "win")
              (begin
                (draw-text "TI VYSHOL" (- (/ w 2) 40) 120 2 (list 0.75 0.9 0.7))
                (draw-text "SHTOLNYA OTPUSTILA TEBYA" (- (/ w 2) 84) 156 1 (list 0.5 0.55 0.45))
                (draw-text "R - SNOVA" (- (/ w 2) 34) 180 1 (list 0.5 0.55 0.45)))
              #f)))))

;; инициализация на верхнем уровне
(sfx-init!)
(print "rlisp-engine ШТОЛЬНЯ: хоррор — фонарь с батареей, монстр-сталкер, 8 записок; весь геймплей на Lisp")
