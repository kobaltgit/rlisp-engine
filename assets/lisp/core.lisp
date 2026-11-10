;; core.lisp — базовая библиотека поверх интерпретатора.
;; Здесь начинается «Lisp-ОС»: всё ниже — обычный Lisp, исполняемый движком.

(define *persistent* (make-table))

;; defpersistent по ТЗ §8: значение переживает перезагрузку окружения.
(defmacro defpersistent (name init)
  `(begin
     (define ,name ,init)
     (table-set! *persistent* ',name ,name)))

(define (caar x) (car (car x)))
(define (cadr x) (car (cdr x)))
(define (caddr x) (car (cdr (cdr x))))
(define (cddr x) (cdr (cdr x)))

(define (map f lst)
  (if (null? lst) '()
      (cons (f (car lst)) (map f (cdr lst)))))

(define (for-each f lst)
  (if (null? lst) #f
      (begin (f (car lst)) (for-each f (cdr lst)))))

(define (filter p lst)
  (if (null? lst) '()
      (if (p (car lst))
          (cons (car lst) (filter p (cdr lst)))
          (filter p (cdr lst)))))

(define (range a b)
  (if (>= a b) '() (cons a (range (+ a 1) b))))

;; dotimes с hygiene- gensym: макрос, разворачивающийся в while
(defmacro dotimes (bind . body)
  (let ((v (car bind)) (n (cadr bind)) (g (gensym)))
    `(let ((,v 0) (,g ,n))
       (while (< ,v ,g)
         ,@body
         (set! ,v (+ ,v 1))))))

(defmacro inc! (x . d)
  `(set! ,x (+ ,x ,(if (null? d) 1 (car d)))))

(defmacro dec! (x . d)
  `(set! ,x (- ,x ,(if (null? d) 1 (car d)))))

(define (clamp v lo hi) (min hi (max lo v)))

;; чтение плоского списка «ключ значение ключ значение …»
(define (get-kv cfg key default)
  (cond ((null? cfg) default)
        ((eq? (car cfg) key) (cadr cfg))
        (else (get-kv (cddr cfg) key default))))

(define (set-kv! t kvs)
  (if (null? kvs) t
      (begin (table-set! t (car kvs) (cadr kvs))
             (set-kv! t (cddr kvs)))))
