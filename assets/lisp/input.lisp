;; input.lisp — Уровень 3: Input DSL (по ТЗ §5.6).
;; Обёртка над сырыми событиями input-poll: хранит состояние между кадрами.

(define *keys* (make-table))         ; клавиша → time-now последнего нажатия
(define *keys-pressed* (make-table)) ; клавиша → #t в кадре нажатия

(define (process-events events)
  (for-each
   (lambda (ev)
     (let ((kind (car ev)) (key (cadr ev)))
       (cond ((eq? kind 'key-down)
              (begin (table-set! *keys* key (time-now))
                     (table-set! *keys-pressed* key #t)))
             ((eq? kind 'key-up)
              (table-set! *keys* key -999.0))
             (else #f))))
   events))

(define (poll-input)
  (process-events (input-poll)))

;; клавиша «залипает» на 0.2с — тап в терминале ощущается как удержание
(define (key-down? k)
  (< (- (time-now) (table-get *keys* k -999.0)) 0.2))

(define (key-pressed? k)
  (table-get *keys-pressed* k #f))

;; вызывается ядром после draw() каждый кадр
(define (frame-end)
  (table-clear! *keys-pressed*))
