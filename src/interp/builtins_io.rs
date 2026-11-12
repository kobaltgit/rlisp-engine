// src/interp/builtins_io.rs — файловый IO для сохранений игр (этап 29).
// save-table / load-table: таблица ↔ JSON-файл. Механика сериализации —
// в save.rs; здесь — привязка к сцене: путь резолвится относительно
// корня сцены, в WASM-сборке ФС нет → честное #f вместо падения.
use std::rc::Rc;

use crate::gc::Gc;
use crate::interp::env::CoreCtx;
use crate::interp::value::{fmt_value, need_args as need, Value};

fn def<F: Fn(&[Value]) -> Result<Value, String> + 'static>(env: Gc, name: &str, f: F) {
    crate::interp::env::env_define(env, name, Value::Native(Rc::new(f), name.to_string()));
}

pub fn register_io_builtins(env: Gc, ctx: &CoreCtx) {
    let lctx = ctx.clone();
    def(env, "save-table", move |args| {
        need(args, 2)?;
        let t = match &args[0] {
            Value::Table(t) => t.clone(),
            other => return Err(format!("save-table: нужна таблица: {}", fmt_value(other))),
        };
        let p = match &args[1] {
            Value::Str(s) => s.clone(),
            other => return Err(format!("save-table: путь — строка: {}", fmt_value(other))),
        };
        let json = crate::interp::save::table_to_json(&t)?;
        #[cfg(target_arch = "wasm32")]
        {
            // в браузере нет файловой системы: честное «не сохранил»
            let _ = (&json, &p);
            Ok(Value::Bool(false))
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let full = crate::interp::loader::resolve_path(&p, &lctx.root);
            if let Some(dir) = full.parent() {
                std::fs::create_dir_all(dir).map_err(|e| format!("save-table: {}", e))?;
            }
            std::fs::write(&full, json).map_err(|e| format!("save-table: {}", e))?;
            Ok(Value::Bool(true))
        }
    });
    let lctx = ctx.clone();
    def(env, "load-table", move |args| {
        need(args, 2)?;
        let t = match &args[0] {
            Value::Table(t) => t.clone(),
            other => return Err(format!("load-table: нужна таблица: {}", fmt_value(other))),
        };
        let p = match &args[1] {
            Value::Str(s) => s.clone(),
            other => return Err(format!("load-table: путь — строка: {}", fmt_value(other))),
        };
        #[cfg(target_arch = "wasm32")]
        {
            let _ = &p;
            return Ok(Value::Bool(false));
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let full = crate::interp::loader::resolve_path(&p, &lctx.root);
            // нет файла — это не ошибка: Lisp решает, есть ли сохранение
            if !full.exists() {
                return Ok(Value::Bool(false));
            }
            let text =
                std::fs::read_to_string(&full).map_err(|e| format!("load-table: {}: {}", p, e))?;
            crate::interp::save::json_to_table(&text, &t)?;
            Ok(Value::Bool(true))
        }
    });
}
