// src/interp/env.rs — Представление таблицы символов, контекст рантайма и реестр загрузки.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use crate::gc::{self, Gc};
use super::Value;

// Этап 2: окружения и замыкания живут в GC-куче (src/gc.rs).
pub type GlobalCell = Rc<RefCell<Gc>>;
pub type LoadRegistry = Rc<RefCell<Vec<PathBuf>>>;

#[derive(Clone)]
pub struct CoreCtx {
    pub global: GlobalCell,
    pub registry: LoadRegistry,
    pub root: PathBuf,
}

// ——————————————————————————— окружения ———————————————————————————

pub fn env_new(parent: Option<Gc>) -> Gc {
    gc::alloc_env(parent)
}

pub fn env_get(env: Gc, name: &str) -> Option<Value> {
    gc::env_get(env, name)
}

pub fn env_set(env: Gc, name: &str, val: Value) -> Result<(), String> {
    gc::env_set(env, name, val)
}

pub fn env_define(env: Gc, name: &str, val: Value) {
    gc::env_define(env, name, val)
}
