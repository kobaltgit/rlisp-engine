# Декомпозиция монолитов и регламент разработки ИИ-агентов (AI Agents Guard)

## 1. Цель и мотивация изменений

Проект **`rlisp-engine`** активно разрабатывается с применением современных ИИ-ассистентов (Cursor, Claude Code, Antigravity, Copilot). По мере реализации этапов 1–24 кодовая база выросла в масштабе, и несколько ключевых файлов превратились в монолиты объёмом до 4,5 тысяч строк:
- `src/render3d.rs` — **4 528 строк**
- `src/interp.rs` — **2 781 строка**
- `src/gpu.rs` — **1 619 строк**
- `src/hal.rs` — **1 545 строк**
- `src/main.rs` — **1 398 строк**
- `src/physics.rs` — **1 361 строка**
- `src/jit.rs` — **1 218 строк**
- `src/simd.rs` — **1 002 строки**
- `src/audio.rs` — **849 строк**
- `src/gpu_3d.rs` — **812 строк**

### Проблемы монолитов для человека и нейросетей:
1. **Переполнение контекстного окна (Context Window Exhaustion):** При анализе файла размером более 1000–4000 строк модель тратит большую часть окна внимания на второстепенный код. Это приводило к тому, что при последующих правках агенты перетирали смежные функции, теряли скобки или повторно раздували файлы.
2. **Размытие ответственности:** Например, в одном файле `hal.rs` находились структуры текстур, монотонные таймеры, звуковой движок WinMM, кодек Base64, ASCII-рендер и 660 строк Lisp-биндингов.
3. **Высокая стоимость чтения и слияния:** Любой точечный фикс в 3D-растеризаторе затрагивал многотысячный файл, создавая частые merge conflicts.

---

## 2. Архитектурные принципы рефакторинга

1. **100% обратная совместимость (Zero-Breaking Change):**
   * Каждый модульный подпакет сохраняет входную точку `mod.rs` (например, `src/gpu/mod.rs` вместо `src/gpu.rs`).
   * Через `pub use self::<submodule>::*;` экспортируются **абсолютно те же самые публичные типы, структуры, функции и константы**.
   * Ни один внешний вызов `crate::gpu::*`, `crate::hal::*`, `crate::render3d::*` в кодовой базе не изменил путь вызова.

2. **Жёсткий лимит размера файлов (File Size Guard < 800 строк):**
   * В корень репозитория добавлен регламент [`AI_AGENTS.md`](../AI_AGENTS.md), явно запрещающий создание и раздувание исходных файлов сверх 800 строк (рекомендуемый ориентир: 150–500 строк).
   * Добавлен автоматический тест [`tests/file_size_lint_test.rs`](../tests/file_size_lint_test.rs), сканирующий все файлы `src/**/*.rs`. Если любой файл превышает 800 строк — тест падает в CI и при локальном запуске `cargo test`.
   * Список временных исключений `PENDING_REFACTORING` в тесте сейчас **полностью пуст** `&[]`.

3. **Соблюдение Resource Guard:**
   * Проект настроен на безопасную сборку без исчерпания памяти и CPU (`-j 1`, `codegen-units = 1`).

---

## 3. Сводка декомпозиции

| Исходный файл | Строк до | Целевой подпакет | Выделенные подмодули |
|---|---|---|---|
| `src/render3d.rs` | **4 528** | `src/render3d/` | `types.rs`, `shading.rs`, `skinning.rs`, `primitives.rs`, `raster.rs` (`tex.rs`, `transparent.rs`), `lisp.rs`, `tests.rs` (`pipeline.rs`, `raster.rs`), `mod.rs` |
| `src/interp.rs` | **2 781** | `src/interp/` | `value.rs`, `env.rs`, `parse.rs`, `eval.rs`, `loader.rs`, `builtins.rs`, `builtins_vec.rs`, `tests.rs`, `mod.rs` |
| `src/gpu.rs` | **1 619** | `src/gpu/` | `types.rs`, `resources.rs`, `wgpu_backend.rs`, `wgpu_2d.rs`, `wgpu_3d.rs`, `wgpu_quad.rs`, `tests.rs`, `mod.rs` |
| `src/hal.rs` | **1 545** | `src/hal/` | `types.rs`, `core.rs`, `lisp.rs` (биндинги Lisp), `tests.rs`, `mod.rs` |
| `src/main.rs` | **1 398** | `src/` (модули) | `src/cli.rs` (парсинг флагов), `src/bench.rs` (бенчмарки 2D/3D/JIT), `src/runner.rs` (игровой цикл и hot-reload), компактный `src/main.rs` (170 строк) |
| `src/physics.rs` | **1 361** | `src/physics/` | `types.rs`, `solver.rs` (солвер Гаусса-Зейделя и динамика), `lisp.rs`, `tests.rs`, `mod.rs` |
| `src/jit.rs` | **1 218** | `src/jit/` | `ir.rs`, `compiler.rs`, `runtime.rs`, `emit.rs`, `tests.rs`, `mod.rs` |
| `src/simd.rs` | **1 002** | `src/simd/` | `types.rs`, `kernels.rs` (scalar/SSE/AVX/AVX-512), `scanner.rs`, `lisp.rs`, `tests.rs`, `mod.rs` |
| `src/audio.rs` | **849** | `src/audio/` | `synth.rs`, `codec.rs` (WAV encoder/decoder), `mixer.rs`, `device.rs` (WinMM), `tests.rs`, `mod.rs` |
| `src/gpu_3d.rs` | **812** | `src/gpu_3d/` | `types.rs`, `shaders.rs` (WGSL шейдеры), `pipeline.rs`, `tests.rs`, `mod.rs` |

Все полученные файлы укладываются в диапазон **50–650 строк**.

---

## 4. Результаты тестов

* `cargo test --workspace -j 1`: **123 passed (122 unit + 1 lint); 0 failed**.
* `cargo test --features wgpu --workspace -j 1`: **125 passed (124 unit + 1 lint); 0 failed**.
* `cargo test --test file_size_lint_test -j 1`: **1 passed; 0 failed** (0 файлов превышают 800 строк).
* Headless smoke-тесты:
  * `cargo run -j 1 -- --script assets/lisp/main.lisp --fast --frames 60` — OK.
  * `cargo run -j 1 -- --script assets/lisp/main_oasis.lisp --fast --frames 60` — OK.
