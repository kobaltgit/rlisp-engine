(load "core.lisp")(load "textures.lisp")(load "font.lisp")(load "graphics.lisp")(load "input.lisp")(load "render3d.lisp")
(load "ruiner.lisp")
(print (string-append "px=" (number->string *px*) " pz=" (number->string *pz*) " pa=" (number->string *pa*)))
;; демон ровно на линии взгляда в 4.5 м
(set! *enemies*
      (cons (cons7 (+ *px* (* 4.5 (cos *pa*))) (+ *pz* (* 4.5 (sin *pa*))) 30 'chase 0.0 *pa* 0.0) '()))
(define (dbg-e) (car *enemies*))
(print (string-append "los=" (number->string (if (los? *px* *pz* (car (dbg-e)) (cadr (dbg-e))) 1 0))))
(print (string-append "ang=" (number->string (- (atan (- (cadr (dbg-e)) *pz*) (- (car (dbg-e)) *px*)) *pa*))))
(fire!)
(print (string-append "hp=" (number->string (caddr (dbg-e))) " ammo=" (number->string *ammo*)))
