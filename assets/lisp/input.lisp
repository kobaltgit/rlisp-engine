;; input.lisp — Уровень 3: Input DSL (по ТЗ §5.6).
;; Обёртка над сырыми событиями input-poll: хранит состояние между кадрами.

(define *keys* (make-table))         ; клавиша → #t (держится) / #f (отпущена)
(define *keys-pressed* (make-table)) ; клавиша → #t в кадре нажатия

(define (process-events events)
  (for-each
   (lambda (ev)
     (let ((kind (car ev)) (key (cadr ev)))
       (cond ((eq? kind 'key-down)
              (begin (table-set! *keys* key #t)
                     (table-set! *keys-pressed* key #t)))
             ((eq? kind 'key-up)
              (table-set! *keys* key #f))
             (else #f))))
   events))

(define (poll-input)
  (process-events (input-poll)))

;; удержание — чистое состояние: #t от key-down до key-up. Окна по времени
;; (0.2с реального, потом 0.5с и 36 sim-кадров) обрывали удержание раньше
;; key-up: в headless-прогоне sim_time идёт медленнее стенного, ходьба
;; замирала. Авто-release для терминального режима делает runner.rs.
(define (key-down? k)
  (table-get *keys* k #f))

(define (key-pressed? k)
  (table-get *keys-pressed* k #f))

;; вызывается ядром после draw() каждый кадр
(define (frame-end)
  (table-clear! *keys-pressed*))
