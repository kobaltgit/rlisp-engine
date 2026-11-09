// src/jit.rs — Этап 3: zero-dependency copy-and-patch JIT для числовых ядер.
//
// Идея (CPython-style): маленькие Lisp-лямбды, тело которых — чистая арифметика
// над числами (+,-,*,/,<,=,>,<=,>=, let/if/and/or, литералы, параметры),
// компилируются в линейный IR (JitIr), а затем в НАТИВНЫЙ машинный код x86-64
// из заранее собранных байтовых шаблонов с пропатченными константами и
// смещениями переходов (copy-and-patch). Код кладётся в RW-память, переводится
// mprotect'ом в RX и вызывается через указатель на функцию.
//
// Модель гардов (стражей): на входе Rust-обёртка проверяет арность и типы
// аргументов (только Int/Float); при провале гарда или unsupported-форме —
// ДЕОПТИМИЗАЦИЯ: прозрачный откат в интерпретатор (src/interp.rs).
// Никаких частично-вычисленных результатов: либо JIT целиком, либо interp.
//
// Хард-требование репозитория — ноль зависимостей: никаких cranelift/llvm.
// mmap/mprotect — прямые Linux-сисвызвы через core::arch::asm! (x86_64-linux).
//
// Статус Этапа 3: СКЕЛЕТ (IR + модель гардов + путь деоптимизации + бенчмарк
// интерпретатора как baseline). Что реально компилируется — см. LIMITATIONS.md
// и fn jit_compile_form ниже; список растёт по мере реализации.

use crate::interp::Value;

// ——————————————————————————— линейный IR ———————————————————————————

/// Одна IR-инструкция стековой машины над f64.
/// Семантика: значение выражения всегда в xmm0; let-слоты и операнды
/// бинарных операций — на f64-стеке (rsp).
#[derive(Debug, Clone, PartialEq)]
pub enum IrOp {
    /// xmm0 = константа из пула doubles
    Const(usize),
    /// xmm0 = параметр №k (из [rdi + 8*k])
    Arg(usize),
    /// push xmm0 на f64-стек (let-слот или левый операнд)
    Push,
    /// pop со f64-стека
    Pop,
    /// xmm0 = [rsp + 8*k] — чтение let-переменной (k от вершины)
    LoadSlot(usize),
    /// xmm0 = xmm0 op xmm1 (xmm1 = снятый со стека левый операнд)
    BinAdd,
    BinSub,
    BinMul,
    BinDiv,
    /// сравнение → булево на f64-стеке (0.0/1.0); xmm1 = левый операнд
    CmpLt,
    CmpEq,
    CmpGt,
    CmpLe,
    CmpGe,
    /// if: снять cond (f64 != 0.0 → then); rel32-переходы патчатся
    Branch { then_fix: usize, else_fix: usize, end_fix: usize },
    /// безусловный переход к end (для and/or/if)
    Jump(usize),
    /// адрес в машинном коде для патча переходов
    Label,
    /// NaN семантика: cond истинно, если xmm0 != 0.0 И не NaN (PF-обработка)
    TruthyCheck,
}

/// Скомпилированная функция: машинный код + метаданные.
pub struct JitFn {
    /// указатель на машинный код: fn(args: *const f64) -> f64
    code: NonNullU8,
    len: usize,
    /// арность, которую проверяет гард
    pub arity: usize,
    /// исходная лямбда для деоптимизации (прозрачный откат в interp)
    pub source: Value,
}

// непрозрачный держатель исполняемой памяти (uninitialized-safe)
struct NonNullU8(usize);

impl JitFn {
    /// Вызов с гарда-проверенными числовыми аргументами.
    /// Гард (типы/арность) проверяется ВЫШЕ по стеку — см. call_jitted().
    pub unsafe fn call(&self, args: *const f64) -> f64 {
        let f: unsafe fn(*const f64) -> f64 = std::mem::transmute(self.code.0);
        f(args)
    }
}

// ——————————————————————————— компилятор ———————————————————————————

/// Попытка JIT-компиляции лямбды. Возвращает None = деоптимизация
/// (unsupported форма, не-числовая арифметика, не x86_64-linux, не хватит памяти).
/// Это ЕДИНСТВЕННАЯ точка входа: вызывающий код обязан уметь исполнить
/// `source` интерпретатором при None — гарантия корректности выше скорости.
pub fn try_compile(lambda: &Value) -> Option<JitFn> {
    // Статус Этапа 3: скелет. Реальная эмиссия машинного кода — jit_emit
    // (см. ветку разработки stage3/jit-simd); пока честно всегда деоптимизируем.
    let _ = lambda;
    None
}

/// Проверка гарда: арность + все аргументы — числа (Int|Float).
/// Числа конвертируются в f64 (Int — с потерей точности > 2^53, что
/// документировано в LIMITATIONS.md как допущение числового ядра).
pub fn guard_args(args: &[Value], arity: usize) -> Option<Vec<f64>> {
    if args.len() != arity {
        return None;
    }
    let mut out = Vec::with_capacity(arity);
    for a in args {
        match a {
            Value::Int(i) => out.push(*i as f64),
            Value::Float(f) => out.push(*f),
            _ => return None,
        }
    }
    Some(out)
}

/// Точка вызова из интерпретатора: гард → нативный вызов, иначе None (деопт).
pub fn call_jitted(f: &JitFn, args: &[Value]) -> Option<Value> {
    let nums = guard_args(args, f.arity)?;
    // safety: машинный код порождён нашим компилятором, читает ровно
    // f.arity doubles по указателю (границы проверены guard_args)
    let r = unsafe { f.call(nums.as_ptr()) };
    // Инвариант: если результат NaN/inf, это честная семантика арифметики f64,
    // совпадающая с интерпретатором (свойственные тесты сверяют interp vs JIT)
    Some(Value::Float(r))
}
