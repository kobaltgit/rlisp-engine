// src/jit/ir.rs — линейный IR для zero-dependency copy-and-patch JIT.

/// Одна IR-инструкция стековой машины над f64.
/// Соглашение: значение выражения — в xmm0; let-слоты и левые операнды
/// бинарных операций лежат на f64-стеке (rsp).
#[derive(Debug, Clone)]
pub enum IrOp {
    /// xmm0 = константа из пула doubles
    Const(usize),
    /// xmm0 = параметр №k (из [rdi + 8*k])
    Arg(usize),
    /// push xmm0 на f64-стек (let-слот или левый операнд)
    Push,
    /// pop верхушки стека в xmm1
    Pop,
    /// xmm0 = [rsp + 8*k] — чтение слота по абсолютному номеру push'а
    LoadSlot(usize),
    /// сдвинуть rsp на 8*k вниз-вверх (дискредитация let-слотов), k>0
    AdjustStack(usize),
    /// xmm0 = xmm0 op xmm1 (xmm1 — снятый со стека ЛЕВЫЙ операнд)
    BinAdd,
    BinSub,
    BinMul,
    BinDiv,
    /// сравнение xmm1 (левый) op xmm0 (правый) → 1.0/0.0 в xmm0
    CmpLt,
    CmpEq,
    CmpGt,
    CmpLe,
    CmpGe,
    /// xmm0 = floor(xmm0) — roundsd imm=1 (этап 15: покрытие eval)
    UnFloor,
    /// xmm0 = sqrt(xmm0) — sqrtsd; отрицательный → NaN → runtime-деоптимизация
    UnSqrt,
    /// xmm0 = |xmm0| — andpd с маской 0x7FFF… из пула констант
    UnAbs,
    /// xmm0 = min(xmm1, xmm0): xmm1 — левый со стека (minsd)
    Min,
    /// xmm0 = max(xmm1, xmm0) (maxsd)
    Max,
    /// ветвление по готовым флагам comisd (cond — сравнение):
    /// переходы патчатся rel32 после эмиссии; pfFix → false-ветка (NaN/unordered),
    /// ccFix → then-ветка (код условия), fallthrough → else-ветка
    Branch {
        /// код условия x86 (0x82 jb — «меньше», 0x84 je, 0x87 ja, 0x86 jbe, 0x83 jae)
        cond: u8,
        then_ir: usize,
        else_ir: usize,
        end_ir: usize,
    },
    /// безусловный переход к IR-индексу
    Jump(usize),
    /// временная заглушка для Branch/Jump (заполняется при компиляции формы)
    Placeholder,
}
