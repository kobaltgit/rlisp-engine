;; main_fox_test.lisp — тест этапа 21: скиннед модель Fox (CC0, из сети),
;; анимация Walk из glTF, отрисовка через gpu3d-mesh-skin.
(load "core.lisp")
(load "font.lisp")
(load "graphics.lisp")
(load "textures.lisp")
(load "render3d.lisp")
(load "input.lisp")
(load "load-gltf.lisp")

(define *fox* (gltf-load-skin "../models/fox.gltf"))
(define *anims* (table-get *fox* 'anims))
(define (probe-anim a)
  (if (null? a)
      #f
      (begin (print (string-append "anim: " (car (car a)) " dur: " (number->string (cadr (car a)))))
             (probe-anim (cdr a)))))
(probe-anim *anims*)
(define *anim* "Walk")
(define *anim-off* #f)

(define *screen-w* 400)
(define *screen-h* 300)

;; земля: большой квад с шахматным цветом по вершинам
(define *ground* (mesh-make (list -8 0 -8   8 0 -8   8 0 8   -8 0 8)
                            (list 0 1 2  0 2 3)))
(define (mat-trans tx ty tz)
  (f32-from-list (list 1 0 0 tx  0 1 0 ty  0 0 1 tz  0 0 0 1)))

(define *ground-vc* (f32-from-list (list 0.55 0.62 0.45 1
                                         0.48 0.56 0.40 1
                                         0.44 0.52 0.38 1
                                         0.51 0.59 0.43 1)))

(define (draw)
  (let* ((t (time-now))
         (eye (list 0 0.75 -2.2))
         (mvp (camera-mvp eye (list 0 0.5 0) 0.95)))
    (gpu3d-clear 0.72 0.82 0.90)
    (gpu3d-view 2.6 1.35 2.6)
    (gpu3d-light 0.4 0.75 0.35 0.50 0.75 0.55 44)
    (gpu3d-fog 0.72 0.82 0.90 0.02)
    (gpu3d-mesh-vc (car *ground*) (make-f32vec 8 0.0) *ground-vc*
                   (cadr *ground*) (mat-identity) mvp 0)
    ;; лиса: скин + Walk; модель в сантиметрах — scale 0.012, ноги на земле
    (let ((model (mat4-mul (mat-trans 0 -0.214 0)
                           (mat-scale 0.012 0.012 0.012))))
      (draw-gltf-skin *fox* (if *anim-off* "" *anim*) (if *anim-off* 0.0 t) model mvp 1.0 1.0 1.0))
    (gpu3d-alpha-blend 1.0)
    (gpu3d-flush-alpha)
    (gpu3d-post 1.05 0.5 0.3 0.03)
    (draw-text "FOX SKIN TEST (stage 21)" 6 5 1 (list 1 1 1))
    (draw-text (string-append "joints "
                              (number->string (length (table-get *fox* 'jnodes)))
                              "  verts "
                              (number->string (table-get *fox* 'verts)))
               6 13 1 (list 0.7 0.9 0.6))
    (gpu-present)))

(define (update dt) #f)
