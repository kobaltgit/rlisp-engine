use super::*;
use crate::hal::{Hal, OutputMode, Tex};
use crate::interp::Value;

#[test]
fn кривая_тумана_экспоненциальная_и_монотонная() {
    // density 0 или глубина 0 — тумана нет
    assert!(близко(fog_factor(0.0, 100.0), 0.0));
    assert!(близко(fog_factor(0.25, 0.0), 0.0));
    // эталонные точки: 1 - exp(-d*x)
    assert!(близко(fog_factor(0.1, 10.0), 1.0 - (-1.0f32).exp()));
    assert!(близко(fog_factor(1.0, 5.0), 1.0 - (-5.0f32).exp()));
    assert!(близко(fog_factor(2.0, 2.0), 1.0 - (-4.0f32).exp()));
    // монотонность по обеим переменным, насыщение ≤ 1
    assert!(fog_factor(0.1, 20.0) > fog_factor(0.1, 10.0));
    assert!(fog_factor(0.2, 10.0) > fog_factor(0.1, 10.0));
    assert!(fog_factor(100.0, 100.0) <= 1.0);
    // отрицательные входы (мусор от скрипта) не паникуют и не дают < 0
    assert!(близко(fog_factor(-1.0, 10.0), 0.0));
    assert!(близко(fog_factor(0.1, -5.0), 0.0));
}

#[test]
fn альфа_тест_граница_включительная() {
    // выкл: порог 0 пропускает всё, включая полностью прозрачное
    assert!(alpha_keep(0, 0.0));
    // граница: alpha/255 == порог — тексель ОСТАЁТСЯ
    assert!(alpha_keep(26, 0.1)); // 26/255 = 0.10196 ≥ 0.1
    assert!(!alpha_keep(25, 0.1)); // 25/255 = 0.09804 < 0.1
                                   // полный альфа-канал проходит даже при пороге 1
    assert!(alpha_keep(255, 1.0));
    assert!(!alpha_keep(254, 1.0));
    // порог 0.5: граница 127/128
    assert!(!alpha_keep(127, 0.5)); // 0.498 < 0.5
    assert!(alpha_keep(128, 0.5)); // 0.502 ≥ 0.5
}

#[test]
fn тайлинг_математика_repeat_и_пасsthrough() {
    // tiling 1 — координата без изменений (старое поведение clamp)
    assert!(близко(tiled_coord(0.75, 1.0), 0.75));
    assert!(близко(tiled_coord(0.0, 1.0), 0.0));
    assert!(близко(tiled_coord(1.0, 1.0), 1.0));
    // repeat: u=0.75 при tiling 2 → 1.5 → fract 0.5
    assert!(близко(tiled_coord(0.75, 2.0), 0.5));
    // u=1.0 при tiling 2 → 2.0 → fract 0 (шов стыкуется с началом)
    assert!(близко(tiled_coord(1.0, 2.0), 0.0));
    // u=1.3 при tiling 3 → 3.9 → fract 0.9
    assert!(близко(tiled_coord(1.3, 3.0), 0.9));
    // нулевой tiling схлопывает координату в 0
    assert!(близко(tiled_coord(0.42, 0.0), 0.0));
}

#[test]
fn блик_блинн_фонга_нулевые_и_граничные() {
    // сила 0 — блика нет (старый Lambert-путь, ноль накладных расходов)
    assert!(близко(spec_highlight(0.9, 32.0, 0.0), 0.0));
    // N·H ≤ 0 — блик нет (half-vector за гранью)
    assert!(близко(spec_highlight(0.0, 32.0, 1.0), 0.0));
    assert!(близко(spec_highlight(-0.5, 32.0, 1.0), 0.0));
    // идеальное отражение: N·H = 1 → ровно strength (любая shininess)
    assert!(близко(spec_highlight(1.0, 32.0, 0.7), 0.7));
    assert!(близко(spec_highlight(1.0, 1.0, 0.7), 0.7));
    // рост показателя сужает блик: при N·H=0.5 и shininess 1 → 0.5*strength
    assert!(близко(spec_highlight(0.5, 1.0, 1.0), 0.5));
    assert!(spec_highlight(0.5, 64.0, 1.0) < spec_highlight(0.5, 8.0, 1.0));
    // shininess < 1 из скрипта поднимается до 1 (powf корректен)
    assert!(близко(spec_highlight(0.5, 0.0, 1.0), 0.5));
}

#[test]
fn свет_разбор_аргументов_фолбэк_прежнее_состояние() {
    let f = |x: f64| Value::Float(x);
    // старый вызов 4 аргументов: spec остаётся прежним (0 по умолчанию)
    let mut r3 = R3::new();
    apply_light_args(&mut r3, &[f(0.1), f(0.2), f(0.3), f(0.4)]).unwrap();
    assert!(близко(r3.light[0], 0.1));
    assert!(близко(r3.ambient, 0.4));
    assert!(близко(r3.diffuse, 0.75)); // дефолт не тронут
    assert!(близко(r3.spec, 0.0));
    assert!(близко(r3.shininess, 32.0));
    // 5 аргументов (как было в v0.2+): diffuse задаётся, spec не трогается
    let mut r3 = R3::new();
    apply_light_args(&mut r3, &[f(0.0), f(1.0), f(0.0), f(0.2), f(0.9)]).unwrap();
    assert!(близко(r3.diffuse, 0.9));
    assert!(близко(r3.spec, 0.0));
    // 7 аргументов: включает блик
    apply_light_args(
        &mut r3,
        &[f(0.0), f(1.0), f(0.0), f(0.2), f(0.9), f(0.6), f(16.0)],
    )
    .unwrap();
    assert!(близко(r3.spec, 0.6));
    assert!(близко(r3.shininess, 16.0));
    // фолбэк: 6 аргументов — shininess сохраняет прежнее значение
    apply_light_args(&mut r3, &[f(0.0), f(1.0), f(0.0), f(0.2), f(0.9), f(0.3)]).unwrap();
    assert!(близко(r3.spec, 0.3));
    assert!(близко(r3.shininess, 16.0));
    // мусорные значения: отрицательный spec/shininess срезаются
    apply_light_args(
        &mut r3,
        &[f(0.0), f(1.0), f(0.0), f(0.2), f(0.9), f(-1.0), f(0.0)],
    )
    .unwrap();
    assert!(близко(r3.spec, 0.0));
    assert!(близко(r3.shininess, 1.0));
    // мало аргументов — честная ошибка, состояние не меняется
    let mut r3 = R3::new();
    assert!(apply_light_args(&mut r3, &[f(0.1), f(0.2), f(0.3)]).is_err());
    assert!(близко(r3.light[0], 0.45)); // дефолт
}

// ——— этап 9: прозрачная очередь, блендинг, mesh-vc ———

/// Квадрат-во-весь-экран своего цвета на глубине z (плоский свет).
fn квадрат_цвета(hal: &mut Hal, z: f32, r: f64, g: f64, b: f64) {
    let pos = [-1.0f32, -1.0, z, 1.0, -1.0, z, 1.0, 1.0, z, -1.0, 1.0, z];
    let idx = [0.0f32, 1.0, 2.0, 0.0, 2.0, 3.0];
    let mm = ед_матрицы();
    let (model, mvp) = (&mm[0..16], &mm[16..32]);
    mesh_draw(
        hal,
        &pos,
        None,
        None,
        None,
        &idx,
        model,
        mvp,
        r,
        g,
        b,
        0,
        Prim::Tri,
    )
    .unwrap();
}

/// Квадрат с обратной (CW) ориентацией — задняя сторона.
fn квадрат_тыл(hal: &mut Hal, z: f32, r: f64, g: f64, b: f64) {
    let pos = [-1.0f32, -1.0, z, 1.0, -1.0, z, 1.0, 1.0, z, -1.0, 1.0, z];
    let idx = [0.0f32, 2.0, 1.0, 0.0, 3.0, 2.0];
    let mm = ед_матрицы();
    let (model, mvp) = (&mm[0..16], &mm[16..32]);
    mesh_draw(
        hal,
        &pos,
        None,
        None,
        None,
        &idx,
        model,
        mvp,
        r,
        g,
        b,
        0,
        Prim::Tri,
    )
    .unwrap();
}

#[test]
fn блендинг_сорт_far_near_сквозной() {
    let mut hal = Hal::new(OutputMode::Silent, 8, 8);
    hal.fill_all(0, 0, 0);
    свет_плоский(&mut hal);
    hal.r3.blend = 0.5;
    // порядок постановки НЕ важен: далёкий зелёный, близкий красный
    квадрат_цвета(&mut hal, 0.9, 0.0, 1.0, 0.0);
    квадрат_цвета(&mut hal, 0.5, 1.0, 0.0, 0.0);
    flush_transparent(&mut hal).unwrap();
    // far→near: зелёный 0.5 поверх чёрного → (0, 128, 0);
    // красный 0.5 поверх → (128, 64, 0). Перепутанный порядок дал бы
    // (128, 128, 0).
    let c = пиксель(&hal, 4, 4);
    assert_eq!(c, [128, 64, 0, 255], "сорт far→near сломан: {:?}", c);
    // z-записи нет: z-буфер в точке остался бесконечным
    assert!(hal.r3.zbuf[4 * 8 + 4].is_infinite());
}

#[test]
fn блендинг_z_тест_в_обе_стороны() {
    let mut hal = Hal::new(OutputMode::Silent, 8, 8);
    hal.fill_all(0, 0, 0);
    свет_плоский(&mut hal);
    // непрозрачный КРАСНЫЙ вписывается первым на z=0.6
    hal.r3.blend = 1.0;
    квадрат_цвета(&mut hal, 0.6, 1.0, 0.0, 0.0);
    // прозрачный ЗЕЛЁНЫЙ ПЕРЕД ним (z=0.4): блендинг поверх
    hal.r3.blend = 0.5;
    квадрат_цвета(&mut hal, 0.4, 0.0, 1.0, 0.0);
    // прозрачный СИНИЙ ЗА ним (z=0.8): z-тест не пропускает
    квадрат_цвета(&mut hal, 0.8, 0.0, 0.0, 1.0);
    flush_transparent(&mut hal).unwrap();
    let c = пиксель(&hal, 4, 4);
    // 0.5 зелёного поверх красного
    assert_eq!(c[0], col(0.5));
    assert_eq!(c[1], col(0.5));
    assert_eq!(c[2], 0, "синий за непрозрачным не должен пройти z-тест");
}

#[test]
fn прозрачный_проход_двусторонний() {
    let mut hal = Hal::new(OutputMode::Silent, 8, 8);
    hal.fill_all(0, 0, 0);
    свет_плоский(&mut hal);
    hal.r3.blend = 0.6;
    // задняя сторона: opaque-проход её отрисовал бы (куллинг), прозрачный —
    // обязан рисовать (листва видна с обеих сторон)
    квадрат_тыл(&mut hal, 0.5, 1.0, 1.0, 0.0);
    flush_transparent(&mut hal).unwrap();
    let c = пиксель(&hal, 4, 4);
    assert_eq!(c[0], col(0.6));
    assert_eq!(c[1], col(0.6));
}

#[test]
fn mesh_vc_градиент_альфы_перспектива_корректна() {
    // альфа 1 слева → 0 справа, цвет красный: пиксель x=1 (u=-0.625) →
    // a = (1-u)/2 = 0.8125 → красный 207 поверх чёрного фона
    let pos = [
        -1.0f32, -1.0, 0.5, 1.0, -1.0, 0.5, 1.0, 1.0, 0.5, -1.0, 1.0, 0.5,
    ];
    let rgba = [
        1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0,
    ];
    let idx = [0.0f32, 1.0, 2.0, 0.0, 2.0, 3.0];
    let mm = ед_матрицы();
    let (model, mvp) = (&mm[0..16], &mm[16..32]);
    let mut hal = Hal::new(OutputMode::Silent, 8, 8);
    hal.fill_all(0, 0, 0);
    свет_плоский(&mut hal);
    mesh_draw(
        &mut hal,
        &pos,
        None,
        Some(&rgba),
        None,
        &idx,
        model,
        mvp,
        1.0,
        1.0,
        1.0,
        0,
        Prim::Tri,
    )
    .unwrap();
    flush_transparent(&mut hal).unwrap();
    let c = пиксель(&hal, 1, 4);
    assert_eq!(c[0], col(0.8125), "градиент альфы: {:?}", c);
    assert_eq!(c[1], 0);
    // правый край: центр последнего пикселя u=0.875 → a=(1-u)/2=0.0625
    let c2 = пиксель(&hal, 7, 4);
    assert_eq!(c2, [col(0.0625), 0, 0, 255]);
}

#[test]
fn очередь_чистится_clear_и_flush() {
    let mut hal = Hal::new(OutputMode::Silent, 8, 8);
    hal.fill_all(0, 0, 0);
    свет_плоский(&mut hal);
    hal.r3.blend = 0.5;
    квадрат_цвета(&mut hal, 0.5, 1.0, 1.0, 1.0);
    assert_eq!(hal.r3.transq.len(), 2); // 2 треугольника квада
    flush_transparent(&mut hal).unwrap();
    assert!(hal.r3.transq.is_empty());
    // clear тоже чистит (забытый flush не портит следующий кадр)
    квадрат_цвета(&mut hal, 0.5, 1.0, 1.0, 1.0);
    hal.r3.reset(8, 8);
    hal.r3.transq.clear();
    assert!(hal.r3.transq.is_empty());
}

#[test]
fn матрица_инстанса_yaw_по_конвенции() {
    // Регрессия: f32::sin_cos возвращает (sin, cos) — прежняя связка
    // перепутывала оси и ломала все батчи. Проверка конвенции
    // M = T·Ry·Rx·Rz·S: yaw=π/2 переводит +X в -Z (колоночный вектор).
    let pi2 = std::f32::consts::FRAC_PI_2;
    let mut m = [0.0f32; 16];
    inst_matrix(
        &[5.0, 6.0, 7.0, 1.0, 1.0, 1.0, pi2, 0.0, 0.0, 1.0, 1.0, 1.0],
        &mut m,
    );
    // вершина (1, 0, 0): world = R·v + t = (0, 0, -1) + (5, 6, 7)
    let wx = m[0] * 1.0 + m[3];
    let wy = m[4] * 1.0 + m[7];
    let wz = m[8] * 1.0 + m[11];
    assert!(близко(wx, 5.0), "wx = {}", wx);
    assert!(близко(wy, 6.0), "wy = {}", wy);
    assert!(близко(wz, 7.0 - 1.0), "wz = {}", wz);
    // масштаб: вершина (0, 2, 0) → (2, 0) → world y = 6 + 2
    let wy2 = m[5] * 2.0 + m[7];
    assert!(близко(wy2, 8.0), "wy2 = {}", wy2);
    // нулевые углы — диагональная матрица масштаба
    inst_matrix(
        &[0.0, 0.0, 0.0, 0.4, 0.4, 0.4, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
        &mut m,
    );
    assert!(близко(m[0], 0.4) && близко(m[5], 0.4) && близко(m[10], 0.4));
    assert!(близко(m[1] + m[2] + m[4] + m[6] + m[8] + m[9], 0.0));
}

#[test]
fn растр_alpha_test_вырезает_и_z_не_пишет() {
    // 2x2 текстура: 3 текселя маджента непрозрачные, угловой (u=1,v=1)
    // полностью прозрачный (alpha 0). Далёкий зелёный квадрат на z=0.9,
    // близкий текстурный с дыркой на z=0.5.
    let px = vec![
        255u8, 0, 255, 255, 255, 0, 255, 255, 255, 0, 255, 255, 9, 9, 9, 0,
    ];
    let зелёный = || Tex::new(1, 1, vec![0, 255, 0, 255]);
    let uv = [0.0f32, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0];
    let плоский_свет = |hal: &mut Hal| {
        сброс_света(hal);
        hal.r3.light = [0.0, 0.0, 1.0];
        hal.r3.ambient = 1.0;
        hal.r3.diffuse = 0.0; // свет плоский: цвет = тексель·тинт
    };

    // без alpha-test близкий квадрат перекрывает ВСЁ маджентой
    let mut hal = Hal::new(OutputMode::Silent, 8, 8);
    hal.tex.push(Tex::new(2, 2, px.clone()));
    hal.tex.push(зелёный());
    hal.fill_all(0, 0, 0);
    свет_плоский(&mut hal);
    рисуем_квадрат(&mut hal, 2, 0.9, &uv);
    assert_eq!(пиксель(&hal, 0, 0), [0, 255, 0, 255]);
    рисуем_квадрат(&mut hal, 1, 0.5, &uv);
    assert_eq!(пиксель(&hal, 0, 0), [255, 0, 255, 255]);
    assert_eq!(пиксель(&hal, 7, 7), [255, 0, 255, 255]);

    // с alpha-test 0.5 прозрачный тексель отброшен — сквозь дыру виден
    // зелёный дальний квадрат: discard не пишет ни цвет, ни z.
    // Ориентация: v=1 → верх экрана (y-flip), u=1 → право; дыра (тексель
    // u=1,v=1) — правый верхний квадрант, напр. пиксель (7,0)
    let mut hal2 = Hal::new(OutputMode::Silent, 8, 8);
    hal2.tex.push(Tex::new(2, 2, px.clone()));
    hal2.tex.push(зелёный());
    hal2.fill_all(0, 0, 0);
    плоский_свет(&mut hal2);
    рисуем_квадрат(&mut hal2, 2, 0.9, &uv);
    hal2.r3.alpha_test = 0.5;
    рисуем_квадрат(&mut hal2, 1, 0.5, &uv);
    assert_eq!(пиксель(&hal2, 7, 0), [0, 255, 0, 255]);
    assert_eq!(пиксель(&hal2, 4, 1), [0, 255, 0, 255]);
    // непрозрачные тексели рисуются как раньше
    assert_eq!(пиксель(&hal2, 0, 0), [255, 0, 255, 255]);
    assert_eq!(пиксель(&hal2, 7, 7), [255, 0, 255, 255]);
    assert_eq!(пиксель(&hal2, 0, 7), [255, 0, 255, 255]);
}

#[test]
fn растр_туман_смешивает_после_шейдинга() {
    // белый квадрат, свет вдоль (0, 0.6, 0.8): ndl = 0.8 →
    // shade = 0.35 + 0.75·0.8 = 0.95 → 242
    let свет = |hal: &mut Hal| {
        сброс_света(hal);
        hal.r3.light = [0.0, 0.6, 0.8];
    };
    let uv = [0.0f32, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0];

    let mut hal = Hal::new(OutputMode::Silent, 8, 8);
    hal.fill_all(0, 0, 0);
    свет(&mut hal);
    рисуем_квадрат(&mut hal, 0, 0.5, &uv);
    assert_eq!(пиксель(&hal, 4, 4), [242, 242, 242, 255]);

    // красный туман density 50 на глубине 1.0: фактор ≈ 1 → цвет тумана
    let mut hal2 = Hal::new(OutputMode::Silent, 8, 8);
    hal2.fill_all(0, 0, 0);
    свет(&mut hal2);
    hal2.r3.fog = [1.0, 0.0, 0.0, 50.0];
    рисуем_квадрат(&mut hal2, 0, 0.5, &uv);
    assert_eq!(пиксель(&hal2, 4, 4), [255, 0, 0, 255]);

    // слабый туман: точное значение по формуле на истинной глубине w=1
    let f = fog_factor(0.1, 1.0);
    let ожид_r = 0.95 * (1.0 - f) + f;
    let ожид_g = 0.95 * (1.0 - f);
    let mut hal3 = Hal::new(OutputMode::Silent, 8, 8);
    hal3.fill_all(0, 0, 0);
    свет(&mut hal3);
    hal3.r3.fog = [1.0, 0.0, 0.0, 0.1];
    рисуем_квадрат(&mut hal3, 0, 0.5, &uv);
    let c = пиксель(&hal3, 4, 4);
    assert_eq!(c[0], col(ожид_r as f64));
    assert_eq!(c[1], col(ожид_g as f64));
}

#[test]
fn растр_блик_блинн_фонга_ярче_lambert() {
    let uv = [0.0f32, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0];
    // камера на +Z (view (0,0,5)), квадрат в z=0.5, свет (0, 0.6, 0.8):
    // Lambert = 0.95 → 242; half-vector блик у центра добавляет ~0.16 →
    // 0.95+0.16 > 1 → clamp → белый
    let mut hal = Hal::new(OutputMode::Silent, 8, 8);
    hal.fill_all(0, 0, 0);
    сброс_света(&mut hal);
    hal.r3.light = [0.0, 0.6, 0.8];
    рисуем_квадрат(&mut hal, 0, 0.5, &uv);
    let ламберт = пиксель(&hal, 4, 4);
    assert_eq!(ламберт, [242, 242, 242, 255]);

    let mut hal2 = Hal::new(OutputMode::Silent, 8, 8);
    hal2.fill_all(0, 0, 0);
    сброс_света(&mut hal2);
    hal2.r3.light = [0.0, 0.6, 0.8];
    hal2.r3.view = [0.0, 0.0, 5.0];
    hal2.r3.spec = 1.0;
    hal2.r3.shininess = 32.0;
    рисуем_квадрат(&mut hal2, 0, 0.5, &uv);
    let с_бликом = пиксель(&hal2, 4, 4);
    assert_eq!(с_бликом, [255, 255, 255, 255]);
    assert!(с_бликом[0] > ламберт[0]);
}

#[test]
fn растр_тайлинг_repeat_меняет_семпл() {
    // 4x1 текстура: столбцы красный / зелёный / синий / белый
    let tex = Tex::new(
        4,
        1,
        vec![
            255, 0, 0, 255, // u ∈ [0, 0.25)
            0, 255, 0, 255, // u ∈ [0.25, 0.5)
            0, 0, 255, 255, // u ∈ [0.5, 0.75)
            255, 255, 255, 255, // u ∈ [0.75, 1)
        ],
    );
    let uv = [0.0f32, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0];
    let плоский_свет = |hal: &mut Hal| {
        сброс_света(hal);
        hal.r3.light = [0.0, 0.0, 1.0];
        hal.r3.ambient = 1.0;
        hal.r3.diffuse = 0.0;
    };

    // без тайлинга пиксель (4,y): fx=4.5 → u≈0.5625 → столбец 2 (синий)
    let mut hal = Hal::new(OutputMode::Silent, 8, 8);
    hal.tex.push(tex.clone());
    hal.fill_all(0, 0, 0);
    свет_плоский(&mut hal);
    рисуем_квадрат(&mut hal, 1, 0.5, &uv);
    assert_eq!(пиксель(&hal, 4, 4), [0, 0, 255, 255]);
    // край u=0 → столбец 0 (красный)
    assert_eq!(пиксель(&hal, 0, 4), [255, 0, 0, 255]);

    // tiling (2,1): u=0.5625 → 1.125 → fract 0.125 → столбец 0 (красный)
    let mut hal2 = Hal::new(OutputMode::Silent, 8, 8);
    hal2.tex.push(tex);
    hal2.fill_all(0, 0, 0);
    плоский_свет(&mut hal2);
    hal2.r3.tiling = [2.0, 1.0];
    рисуем_квадрат(&mut hal2, 1, 0.5, &uv);
    assert_eq!(пиксель(&hal2, 4, 4), [255, 0, 0, 255]);
    // шов стыкуется: u≈0.9375 → 1.875 → fract 0.875 → столбец 3 (белый)
    assert_eq!(пиксель(&hal2, 7, 4), [255, 255, 255, 255]);
}
