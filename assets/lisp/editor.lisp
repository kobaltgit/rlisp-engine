;; editor.lisp — Уровень 3: мост редактора и MCP (v0.3).
;; Редактор (--serve) и ИИ-агент (--mcp) работают со сценой ТОЛЬКО через Lisp:
;;   (editor-scene)  — сцена как список таблиц (JSON-объекты) для иерархии;
;;   (mcp-spawn …)   — спавн сущности;
;;   (mcp-set …)     — правка свойства живой сущности;
;;   (mcp-remove …)  — удаление.
;; Игра регистрирует хуки (scene-entities / game-spawn / game-entity /
;; game-remove!) — у 2D и 3D игр они свои. Нет хука — инструмент честно
;; отвечает, что в этой игре его нет.

(define (editor-scene)
  (if (defined? 'scene-entities)
      (scene-entities)
      '()))

(define (mcp-spawn kind)
  (if (defined? 'game-spawn)
      (game-spawn kind)
      (error "в этой игре нет game-spawn")))

(define (mcp-spawn-at kind x z)
  (if (defined? 'game-spawn-at)
      (game-spawn-at kind x z)
      (mcp-spawn kind)))

(define (mcp-entity id)
  (if (defined? 'game-entity)
      (game-entity id)
      (error "в этой игре нет game-entity")))

;; свойство сущности: (mcp-set 3 'x 12.5)
(define (mcp-set id prop val)
  (table-set! (mcp-entity id) prop val))

(define (mcp-remove id)
  (if (defined? 'game-remove!)
      (game-remove! id)
      (error "в этой игре нет game-remove!")))

;; хелпер: таблица-объект из плоского списка «ключ значение …»
(define (obj . kvs)
  (let ((t (make-table)))
    (set-kv! t kvs)
    t))
