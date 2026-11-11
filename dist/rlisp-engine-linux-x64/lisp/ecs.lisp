;; ecs.lisp — Уровень 3: ECS/Scene DSL (по ТЗ §5.3).
;; Сущности — хеш-таблицы компонентов, системы — Lisp-функции.
;; HAL не знает ни про entities, ни про components.

(defpersistent *entities* (make-table)) ; id → сущность (таблица)
(defpersistent *next-id* 0)

(define *components* (make-table))      ; имя компонента → дефолты (плоский список)
(define *systems* '())                  ; (имя . функция), порядок объявления

(defmacro defcomponent (name . defaults)
  `(table-set! *components* ',name ',defaults))

(defmacro defsystem (name args . body)
  `(begin
     (define ,name (lambda ,args ,@body))
     (set! *systems* (cons (list ',name ,name) *systems*))))

(define (make-entity . comps)
  (let ((id *next-id*) (e (make-table)))
    (for-each (lambda (c) (set-kv! e (table-get *components* c '()))) comps)
    (table-set! *entities* id e)
    (set! *next-id* (+ id 1))
    id))

(define (entity-get id) (table-get *entities* id #f))

(define (entity-ref id k default)
  (let ((e (table-get *entities* id #f)))
    (if e (table-get e k default) default)))

(define (entity-set! id k v)
  (let ((e (table-get *entities* id #f)))
    (if e (table-set! e k v) #f)))

(define (entity-remove! id) (table-remove! *entities* id))

(define (entity-count) (table-count *entities*))

;; обход по снапшоту — безопасно удалять сущности внутри цикла
(define (for-each-entity f)
  (for-each (lambda (entry) (f (car entry) (cadr entry)))
            (table-entries *entities*)))

;; все системы по всем сущностям: (sys dt id entity)
(define (run-systems dt)
  (for-each
   (lambda (sys) (for-each-entity (lambda (id e) ((cadr sys) dt id e))))
   *systems*))
