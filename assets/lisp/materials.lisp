;; materials.lisp — Уровень 3: реестр материалов поверх состояния «GPU» (этап 7).
;; Материал — именованный набор свойств фиксированного конвейера:
;;   :texture    id текстуры (или #f — берётся из draw-вызова)
;;   :tiling     (u v) — множители UV перед семплингом (repeat-текстуры)
;;   :spec       0..1 — сила блика Blinn-Phong (0 = чистый Lambert)
;;   :shininess  показатель блика: больше — пятно уже (дефолт 32)
;;   :alpha-test 0..1 — порог cutout: тексели с alpha < порога отбрасываются
;;   :shading    'flat | 'smooth — режим затенения (дефолт 'flat)
;;   :filter     'nearest | 'bilinear — фильтрация текстуры (дефолт 'nearest)
;;   :mipmaps    #t | #f — авто-выбор уровней детализации (дефолт #f)
;; Состояние «GPU» — машина состояний (как fixed-function): use-material!
;; применяет свойства, material-reset! возвращает нейтральные значения.
;; Туман (gpu3d-fog) — свойство СЦЕНЫ, не материала: ставится отдельно.
;;
;; defmaterial — ФУНКЦИЯ: ключи-символы и списки передаются цитированными
;; (значения-выражения вычисляются как обычно):
;;   (defmaterial 'трава ':tiling '(2 4) ':spec 0.6 ':shininess 32 ':alpha-test 0.4)

(define *materials* (make-table))

;; (defmaterial имя ':prop значение ...) — создать/заменить материал.
(define (defmaterial name . props)
  (let ((m (make-table)))
    (define (пар rest)
      (if (null? rest)
          #f
          (begin
            (table-set! m (car rest) (cadr rest))
            (пар (cddr rest)))))
    (пар props)
    (table-set! *materials* name m)
    m))

;; (material-texture имя) → id текстуры материала или #f.
;; draw-хелперы используют её как переопределение tex-аргумента.
(define (material-texture name)
  (let ((m (table-get *materials* name #f)))
    (if m (table-get m ':texture #f) #f)))

;; (material-get имя ключ [дефолт]) — прямой доступ к свойству (HUD/отладка).
(define (material-get name key . opt)
  (let ((m (table-get *materials* name #f)))
    (if m
        (apply table-get m key opt)
        (if (null? opt) #f (car opt)))))

;; (use-material! имя) — применить материал к состоянию «GPU».
;; Свет (направление/ambient/diffuse) не трогается: берутся ТЕКУЩИЕ значения
;; из (gpu3d-state), поверх них ставятся spec/shininess материала.
;; Возвращает имя (удобно в цепочках).
(define (use-material! name)
  (let ((m (table-get *materials* name #f)))
    (if m
        (begin
          ;; тайлинг: (:tiling (u v)) или нейтральный (1 1)
          (let ((tl (table-get m ':tiling #f)))
            (if tl
                (gpu3d-mat-tiling (car tl) (cadr tl))
                (gpu3d-mat-tiling 1 1)))
          ;; cutout: 0 = выкл
          (gpu3d-alpha-test (table-get m ':alpha-test 0))
          ;; затенение: переопределяется только если материал явно задал :shading
          (let ((sh (table-get m ':shading #f)))
            (if sh (gpu3d-shading sh) #f))
          ;; фильтрация текстуры: переопределяется только если материал явно задал :filter
          (let ((flt (table-get m ':filter #f)))
            (if flt (gpu3d-filter flt) #f))
          ;; пирамида мипмапов: переопределяется только если материал явно задал :mipmaps
          (let ((mp (table-get m ':mipmaps #f)))
            (if mp (gpu3d-mipmaps mp) #f))
          ;; блик поверх текущего света; нет :spec → чистый Lambert
          (let ((st (gpu3d-state)))
            (let ((l (table-get st 'light)))
              (gpu3d-light (car l) (cadr l) (caddr l)
                           (table-get st 'ambient)
                           (table-get st 'diffuse)
                           (table-get m ':spec 0)
                           (table-get m ':shininess 32))))
          name)
        (error "use-material!: нет материала" name))))

;; (material-reset!) — нейтральное состояние материалов конвейера:
;; без тайлинга, без cutout, без блика. Направление света и ambient/diffuse
;; остаются как были (они — свойство сцены, не материала).
(define (material-reset!)
  (gpu3d-mat-tiling 1 1)
  (gpu3d-alpha-test 0)
  (let ((st (gpu3d-state)))
    (let ((l (table-get st 'light)))
      (gpu3d-light (car l) (cadr l) (caddr l)
                   (table-get st 'ambient)
                   (table-get st 'diffuse)
                   0
                   32)))
  #t)
