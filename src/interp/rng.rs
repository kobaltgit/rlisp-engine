// src/interp/rng.rs — единственный источник случайности в движке.
//
// Вынесено из builtins.rs по правилу §1 AI_AGENTS.md (файл близился к лимиту)
// и по существу: случайность — это то, что делает прогон воспроизводимым или
// нет, поэтому у неё отдельный, маленький и проверяемый модуль.
//
// Контракт: (random-seed n) фиксирует состояние; любой прогон с одинаковым
// seed, одинаковым вводом и фиксированным dt обязан дать тот же кадр
// (проверка — hash-цепочка в src/replay.rs).

use std::sync::atomic::{AtomicU64, Ordering};

static RND_STATE: AtomicU64 = AtomicU64::new(0);
/// Seed, с которым прогон начался: его пишем в файл записи, а не текущее
/// состояние ГПСЧ (иначе запись, начатая с середины, не воспроизводится).
static INITIAL_SEED: AtomicU64 = AtomicU64::new(0);

/// Линейный конгруэнтный шаг (как было с v0.1). Значение в [0,1).
pub fn next_random() -> f64 {
    let mut x = RND_STATE.load(Ordering::Relaxed);
    if x == 0 {
        x = time_seed();
    }
    x = x
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    RND_STATE.store(x, Ordering::Relaxed);
    (x >> 11) as f64 / (1u64 << 53) as f64
}

fn time_seed() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x9E37_79B9_7F4A_7C15)
        | 1
}

/// Зафиксировать seed ровно тем числом, что передали (0 — «взять из времени»).
pub fn set_seed(s: u64) {
    let s = if s == 0 { time_seed() } else { s };
    RND_STATE.store(s, Ordering::Relaxed);
    // первый установленный seed и есть seed прогона
    let _ = INITIAL_SEED.compare_exchange(0, s, Ordering::Relaxed, Ordering::Relaxed);
}

/// Текущее состояние ГПСЧ — то, что попадает в файл прогона.
pub fn seed() -> u64 {
    RND_STATE.load(Ordering::Relaxed)
}

/// Ненулевой seed: инициализировать при первом обращении и вернуть.
pub fn ensure_seed() -> u64 {
    let s = RND_STATE.load(Ordering::Relaxed);
    if s == 0 {
        let t = time_seed();
        RND_STATE.store(t, Ordering::Relaxed);
        let _ = INITIAL_SEED.compare_exchange(0, t, Ordering::Relaxed, Ordering::Relaxed);
        t
    } else {
        s
    }
}

/// Seed, с которого начался прогон (0 — ещё не инициализирован).
pub fn initial_seed() -> u64 {
    let init = INITIAL_SEED.load(Ordering::Relaxed);
    if init == 0 {
        ensure_seed();
        INITIAL_SEED.load(Ordering::Relaxed)
    } else {
        init
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn одинаковый_seed_даёт_одинаковую_последовательность() {
        set_seed(20261014);
        let a: Vec<f64> = (0..16).map(|_| next_random()).collect();
        set_seed(20261014);
        let b: Vec<f64> = (0..16).map(|_| next_random()).collect();
        assert_eq!(a, b);
        assert!(a.iter().all(|v| (0.0..1.0).contains(v)));
    }

    #[test]
    fn разный_seed_даёт_разную_последовательность() {
        set_seed(1);
        let a: Vec<f64> = (0..8).map(|_| next_random()).collect();
        set_seed(2);
        let b: Vec<f64> = (0..8).map(|_| next_random()).collect();
        assert_ne!(a, b);
    }

    #[test]
    fn initial_seed_фиксируется_первым() {
        // INITIAL_SEED — на процесс (как и сам прогон), поэтому проверяем
        // инвариант: дальнейшие смены seed не двигают seed начала прогона
        set_seed(424242);
        let first = initial_seed();
        assert_ne!(first, 0);
        let _ = next_random();
        set_seed(777);
        assert_eq!(initial_seed(), first, "seed прогона не должен меняться");
        assert_eq!(seed(), 777, "а рабочий seed — меняется");
    }

    #[test]
    fn seed_не_искажается() {
        set_seed(20261014);
        assert_eq!(seed(), 20261014);
        set_seed(0);
        assert!(seed() != 0);
    }
}
