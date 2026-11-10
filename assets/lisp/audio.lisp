;; audio.lisp — Высокоуровневый DSL для аудиосистемы rlisp-engine.
;; Процедурные звуковые эффекты (SFX), пресеты (лазер, взрыв, прыжок, монета),
;; управление воспроизведением, громкостью и музыкой.

(define *audio-sounds* (make-table))
(define *current-music-voice* #f)

;; Регистрация WAV-файла в реестре
(define (defsound name path)
  (let ((id (sound-load path)))
    (table-set! *audio-sounds* name id)
    id))

;; Регистрация процедурного чиптюн-звука в реестре
(define (defsound-synth name wave freq duration . opt)
  (let ((att (if (> (length opt) 0) (car opt) 0.01))
        (dec (if (> (length opt) 1) (cadr opt) 0.05))
        (sus (if (> (length opt) 2) (caddr opt) 0.5))
        (rel (if (> (length opt) 3) (car (cddr (cdr opt))) 0.1))
        (slide (if (> (length opt) 4) (cadr (cddr (cdr opt))) 0.0))
        (duty (if (> (length opt) 5) (caddr (cddr (cdr opt))) 0.5)))
    (let ((id (sound-synth wave freq duration att dec sus rel slide duty)))
      (table-set! *audio-sounds* name id)
      id)))

;; Воспроизведение звука по имени (символу) или ID
(define (play-sfx name . opt)
  (let ((id (if (number? name)
                name
                (table-get *audio-sounds* name #f))))
    (if id
        (let ((vol (if (> (length opt) 0) (car opt) 1.0))
              (pitch (if (> (length opt) 1) (cadr opt) 1.0))
              (pan (if (> (length opt) 2) (caddr opt) 0.0))
              (loop? (if (> (length opt) 3) (car (cddr (cdr opt))) #f)))
          (sound-play id vol pitch pan loop?))
        #f)))

;; Остановка конкретного голоса
(define (stop-voice voice-id)
  (if voice-id
      (sound-stop voice-id)
      #f))

;; Остановка всех звуков
(define (stop-all-audio)
  (sound-stop-all)
  (set! *current-music-voice* #f))

;; Воспроизведение зацикленной фоновой музыки / амбиента
(define (play-music name . opt)
  (let ((vol (if (> (length opt) 0) (car opt) 0.8)))
    (if *current-music-voice*
        (sound-stop *current-music-voice*))
    (set! *current-music-voice* (play-sfx name vol 1.0 0.0 #t))
    *current-music-voice*))

;; Остановка музыки
(define (stop-music)
  (if *current-music-voice*
      (begin
        (sound-stop *current-music-voice*)
        (set! *current-music-voice* #f))))

;; Управление мастер-громкостью
(define (audio-set-master-volume! vol)
  (sound-master-volume vol))

(define (audio-get-master-volume)
  (sound-master-volume))

;; ——— Классические пресеты SFX (sfxr / PICO-8 стиль) ———

;; Выстрел лазера: нисходящий пилообразный свист
(define (audio-preset-laser name . opt)
  (let ((freq (if (> (length opt) 0) (car opt) 880.0)))
    (defsound-synth name 'sawtooth freq 0.15 0.005 0.05 0.3 0.08 -1800.0 0.5)))

;; Взрыв: низкочастотный шум с долгим затуханием
(define (audio-preset-explosion name)
  (defsound-synth name 'noise 160.0 0.45 0.005 0.15 0.4 0.25 -200.0 0.5))

;; Попадание / урон: резкий спад частоты
(define (audio-preset-hit name)
  (defsound-synth name 'square 320.0 0.12 0.005 0.04 0.2 0.06 -600.0 0.3))

;; Прыжок: восходящий свист
(define (audio-preset-jump name)
  (defsound-synth name 'square 220.0 0.18 0.01 0.06 0.5 0.1 800.0 0.5))

;; Монета / бонус: яркий двухтоновый колокольчик
(define (audio-preset-coin name)
  (defsound-synth name 'sine 987.0 0.2 0.005 0.04 0.6 0.12 400.0 0.5))

;; Двигатель корабля: низкий треугольный гул
(define (audio-preset-engine name)
  (defsound-synth name 'triangle 85.0 0.35 0.05 0.1 0.8 0.15 0.0 0.5))
