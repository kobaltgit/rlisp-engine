// src/interp/loader.rs — Подсистема загрузки файлов, виртуальная файловая система и I/O sink.

use std::path::{Path, PathBuf};

use super::{eval, parse_program, CoreCtx};
use crate::gc;

// ——————————————————————————— загрузка файлов ———————————————————————————

pub fn resolve_path(p: &str, root: &Path) -> PathBuf {
    let path = Path::new(p);
    if path.is_absolute() {
        return path.to_path_buf();
    }
    let c1 = root.join(path);
    if c1.exists() {
        return c1;
    }
    let c2 = PathBuf::from(p);
    if c2.exists() {
        return c2;
    }
    c1
}

/// Модели, вшитые в бинарник (этап 22): браузерная песочница не имеет
/// файловой системы — те же строки-пути отдают содержимое из embed.
/// Нативные запуски читают реальные файлы (embed — фолбэк, если файла нет).
pub static EMBEDDED_FILES: &[(&str, &[u8])] = &[
    (
        "../models/fox.gltf",
        include_bytes!("../../assets/models/fox.gltf"),
    ),
    (
        "../models/Fox.bin",
        include_bytes!("../../assets/models/Fox.bin"),
    ),
    (
        "../models/Texture.png",
        include_bytes!("../../assets/models/Texture.png"),
    ),
];

pub fn embedded_lookup(name: &str) -> Option<&'static [u8]> {
    EMBEDDED_FILES
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, b)| *b)
}

/// wasm-песочница: Lisp-исходники, встроенные в бинарь (fs недоступен).
/// static + экспорт-зонд в wasm.rs держат данные живыми против DCE.
#[cfg(target_arch = "wasm32")]
pub static EMBEDDED_LISP: &[(&str, &str)] = &[
    ("core.lisp", include_str!("../../assets/lisp/core.lisp")),
    ("ecs.lisp", include_str!("../../assets/lisp/ecs.lisp")),
    ("editor.lisp", include_str!("../../assets/lisp/editor.lisp")),
    ("font.lisp", include_str!("../../assets/lisp/font.lisp")),
    ("game.lisp", include_str!("../../assets/lisp/game.lisp")),
    ("game3d.lisp", include_str!("../../assets/lisp/game3d.lisp")),
    (
        "graphics.lisp",
        include_str!("../../assets/lisp/graphics.lisp"),
    ),
    ("input.lisp", include_str!("../../assets/lisp/input.lisp")),
    ("island.lisp", include_str!("../../assets/lisp/island.lisp")),
    (
        "load-gltf.lisp",
        include_str!("../../assets/lisp/load-gltf.lisp"),
    ),
    ("main.lisp", include_str!("../../assets/lisp/main.lisp")),
    ("main3d.lisp", include_str!("../../assets/lisp/main3d.lisp")),
    (
        "main_island.lisp",
        include_str!("../../assets/lisp/main_island.lisp"),
    ),
    (
        "main_physpoly.lisp",
        include_str!("../../assets/lisp/main_physpoly.lisp"),
    ),
    (
        "materials.lisp",
        include_str!("../../assets/lisp/materials.lisp"),
    ),
    ("audio.lisp", include_str!("../../assets/lisp/audio.lisp")),
    (
        "../assets/lisp/audio.lisp",
        include_str!("../../assets/lisp/audio.lisp"),
    ),
    ("oasis.lisp", include_str!("../../assets/lisp/oasis.lisp")),
    (
        "../assets/lisp/oasis.lisp",
        include_str!("../../assets/lisp/oasis.lisp"),
    ),
    (
        "main_oasis.lisp",
        include_str!("../../assets/lisp/main_oasis.lisp"),
    ),
    (
        "../assets/lisp/main_oasis.lisp",
        include_str!("../../assets/lisp/main_oasis.lisp"),
    ),
    (
        "shtolnya.lisp",
        include_str!("../../assets/lisp/shtolnya.lisp"),
    ),
    (
        "../assets/lisp/shtolnya.lisp",
        include_str!("../../assets/lisp/shtolnya.lisp"),
    ),
    (
        "main_shtolnya.lisp",
        include_str!("../../assets/lisp/main_shtolnya.lisp"),
    ),
    (
        "../assets/lisp/main_shtolnya.lisp",
        include_str!("../../assets/lisp/main_shtolnya.lisp"),
    ),
    (
        "phys_poly.lisp",
        include_str!("../../assets/lisp/phys_poly.lisp"),
    ),
    (
        "physics.lisp",
        include_str!("../../assets/lisp/physics.lisp"),
    ),
    (
        "render3d.lisp",
        include_str!("../../assets/lisp/render3d.lisp"),
    ),
    (
        "textures.lisp",
        include_str!("../../assets/lisp/textures.lisp"),
    ),
];

#[cfg(target_arch = "wasm32")]
pub fn wasm_embedded_file(name: &str) -> &'static str {
    EMBEDDED_LISP
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, s)| *s)
        .unwrap_or("")
}

#[cfg(target_arch = "wasm32")]
fn wasm_embedded(name: &str) -> Option<&'static str> {
    EMBEDDED_LISP
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, s)| *s)
}

pub fn load_file(path: &Path, ctx: &CoreCtx) -> Result<(), String> {
    #[cfg(target_arch = "wasm32")]
    {
        if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
            if let Some(src) = wasm_embedded(name) {
                return load_source(src, name, ctx);
            }
        }
        return Err(format!(
            "load: файл не встроен в песочницу: {}",
            path.display()
        ));
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let src =
            std::fs::read_to_string(path).map_err(|e| format!("load {}: {}", path.display(), e))?;
        {
            let mut reg = ctx.registry.borrow_mut();
            let canon = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
            if !reg.contains(&canon) {
                reg.push(canon);
            }
        }
        load_source(&src, &path.display().to_string(), ctx)
    }
}

fn load_source(src: &str, label: &str, ctx: &CoreCtx) -> Result<(), String> {
    let prog = parse_program(src).map_err(|e| format!("{}: {}", label, e))?;

    // Формы живут до конца загрузки (могут содержать Cons-литералы) —
    // защищаем их от сборки на границах форм.
    let _pin = gc::pin(prog.clone());
    let g = *ctx.global.borrow();
    for (i, e) in prog.iter().enumerate() {
        eval(e, g, 0).map_err(|err| format!("{}: форма {}: {}", label, i + 1, err))?;
        // граница формы: авто-сборка по порогу/флагу (стек eval пуст)
        gc::maybe_collect(&[g], &[]);
    }
    Ok(())
}

// ——————————————————————————— I/O sink и режимы вывода ———————————————————————————

/// v0.3: true в --mcp — Lisp-вывод (print/display) уходит в stderr,
/// чтобы не портить JSON-RPC поток протокола.
static MCP_QUIET: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn set_mcp_quiet(on: bool) {
    MCP_QUIET.store(on, std::sync::atomic::Ordering::Relaxed);
}

#[cfg(target_arch = "wasm32")]
thread_local! {
    static OUT_SINK: std::cell::RefCell<String> = std::cell::RefCell::new(String::new());
}

/// wasm-песочница: дренировать накопленный вывод print/display/newline
#[cfg(target_arch = "wasm32")]
pub fn take_out_sink() -> String {
    OUT_SINK.with(|o| std::mem::take(&mut *o.borrow_mut()))
}

#[cfg(target_arch = "wasm32")]
pub fn out_line_stdout(s: &str) {
    OUT_SINK.with(|o| o.borrow_mut().push_str(s));
}

#[cfg(not(target_arch = "wasm32"))]
pub fn out_line_stdout(s: &str) {
    if MCP_QUIET.load(std::sync::atomic::Ordering::Relaxed) {
        eprint!("{}", s);
    } else {
        print!("{}", s);
        let _ = std::io::Write::flush(&mut std::io::stdout());
    }
}
