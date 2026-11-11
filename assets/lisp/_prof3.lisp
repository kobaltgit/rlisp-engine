;; _prof3: пустой кадр + пост (удалить)
(load "core.lisp")(load "render3d.lisp")
(define (draw) (gpu3d-clear 0.5 0.6 0.7) (gpu3d-post 1.08 0.55 0.30 0.030) (gpu-present))
(define (update dt) #t)
