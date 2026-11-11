// src/jit/runtime.rs — рантайм JIT: исполняемый код, входные гарды и кэш.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::gc;
use crate::interp::Value;
use super::compiler::compile;
use super::emit;

/// Скомпилированная функция: машинный код + метаданные.
pub struct JitFn {
    /// адрес машинного кода (исполняемая mmap-страница)
    pub(crate) code: usize,
    pub(crate) len: usize,
    /// арность, которую проверяет входной гард
    pub arity: usize,
    /// константы пула (для отладки/дампов)
    pub consts: Vec<f64>,
}

impl Drop for JitFn {
    fn drop(&mut self) {
        emit::release_code(self.code, self.len);
    }
}

impl JitFn {
    /// Вызов машинного кода: fn(args: *const f64) -> f64.
    /// Гард (типы/арность) обязан быть проверен call_jitted().
    pub unsafe fn call(&self, args: *const f64) -> f64 {
        let f: unsafe fn(*const f64) -> f64 = std::mem::transmute(self.code);
        f(args)
    }

    /// Адрес машинного кода (для дампа/дизассемблирования в тестах).
    pub fn code_addr(&self) -> usize {
        self.code
    }

    /// Длина машинного кода в байтах (включая пул констант).
    pub fn code_len(&self) -> usize {
        self.len
    }
}

thread_local! {
    /// Кэш компиляции по (idx, gen) GC-объекта лямбды: Some — скомпилирована,
    /// None — деоптимизация навсегда (нет повторных попыток на горячем пути).
    static JIT_CACHE: RefCell<HashMap<(u32, u32), Option<Rc<JitFn>>>> = RefCell::new(HashMap::new());
}

/// Точка входа из интерпретатора: компиляция (кэш) + входные гарды + вызов.
/// None на любом этапе = прозрачная деоптимизация в интерпретатор.
pub fn try_call_lambda(g: gc::Gc, args: &[Value]) -> Option<Value> {
    JIT_CACHE.with(|cache| {
        let mut m = cache.borrow_mut();
        let key = (g.idx, g.gen);
        let compiled = m
            .entry(key)
            .or_insert_with(|| compile(&Value::Lambda(g)).map(Rc::new));
        match compiled {
            Some(jf) => call_jitted(jf, args),
            None => None,
        }
    })
}

/// Проверка входного гарда: арность + все аргументы — числа (Int|Float).
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

/// Вызов скомпилированной функции с гардами. Int-аргументы конвертируются
/// в f64 (это совпадает с семантикой интерпретатора: его арифметика внутри
/// всё равно считается в f64 — см. def "+"; расхождение возможно только за
/// границей точности 2^53 — задокументировано в LIMITATIONS.md).
pub fn call_jitted(f: &JitFn, args: &[Value]) -> Option<Value> {
    let nums = guard_args(args, f.arity)?;
    let r = unsafe { f.call(nums.as_ptr()) };
    if r.is_nan() {
        // протокол runtime-деоптимизации: машинный код встретил деление на
        // ноль (интерпретатор в этом месте возвращает ошибку) — откат в interp
        return None;
    }
    Some(Value::Float(r))
}
