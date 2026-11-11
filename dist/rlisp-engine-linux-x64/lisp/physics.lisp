;; physics.lisp — Уровень 3: Physics DSL (по ТЗ §5.2).
;; AABB-коллизии и интегрирование. Горячий путь — пакетное интегрирование
;; через SIMD-примитивы HAL (vec-add/vec-scale): одна операция двигает ВСЕ тела.
;; HAL не содержит physics-step — смена солвера это правка этого файла.

(define (aabb-collide? ax ay aw ah bx by bw bh)
  (and (< ax (+ bx bw))
       (> (+ ax aw) bx)
       (< ay (+ by bh))
       (> (+ ay ah) by)))

;; направление из (sx sy) в (tx ty), нормированное — через vec-normalize (SIMD)
(define (dir-to sx sy tx ty)
  (let* ((v (f32-from-list (list (- tx sx) (- ty sy))))
         (n (vec-normalize v)))
    (list (f32ref n 0) (f32ref n 1))))

;; === Пакетное интегрирование: pos += vel * dt для всех тел одной SIMD-операцией ===
;; ids — список id сущностей ECS с полями x y vx vy.
(define (integrate-batch! ids dt)
  (if (null? ids) #f
      (let* ((n (length ids))
             (pos (make-f32vec (* 2 n) 0.0))
             (vel (make-f32vec (* 2 n) 0.0)))
        (let ((i 0))
          (for-each
           (lambda (id)
             (f32set! pos (* 2 i) (entity-ref id 'x 0.0))
             (f32set! pos (+ (* 2 i) 1) (entity-ref id 'y 0.0))
             (f32set! vel (* 2 i) (entity-ref id 'vx 0.0))
             (f32set! vel (+ (* 2 i) 1) (entity-ref id 'vy 0.0))
             (set! i (+ i 1)))
           ids)
          ;; SIMD-вызов №1: pos = vec-add pos (vec-scale vel dt)
          (let ((np (vec-add pos (vec-scale vel dt))))
            (let ((j 0))
              (for-each
               (lambda (id)
                 (entity-set! id 'x (f32ref np (* 2 j)))
                 (entity-set! id 'y (f32ref np (+ (* 2 j) 1)))
                 (set! j (+ j 1)))
               ids))
            #t)))))
