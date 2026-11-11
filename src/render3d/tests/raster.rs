use super::*;
use crate::hal::{Hal, OutputMode, Tex};

#[test]
fn smooth_normals() {
    // Квадрат во весь экран (8x8) на z=0.5:
    // Вершины:
    // 0: (-1, -1), 1: (1, -1), 2: (1, 1), 3: (-1, 1)
    // Нормали вершин: левые вершины (0, 3) смотрят вбок (-1, 0, 0),
    // правые вершины (1, 2) смотрят вперед (0, 0, 1).
    // Источник света прямо спереди: light = [0, 0, 1].
    // ambient = 0.2, diffuse = 0.8.
    let pos = [
        -1.0f32, -1.0, 0.5, 1.0, -1.0, 0.5, 1.0, 1.0, 0.5, -1.0, 1.0, 0.5,
    ];
    let nrm = [
        -1.0f32, 0.0, 0.0, // v0
        0.0, 0.0, 1.0, // v1
        0.0, 0.0, 1.0, // v2
        -1.0, 0.0, 0.0, // v3
    ];
    let idx = [0.0f32, 1.0, 2.0, 0.0, 2.0, 3.0];
    let mm = ед_матрицы();
    let (model, mvp) = (&mm[0..16], &mm[16..32]);

    // 1. В режиме flat: нормаль треугольников берётся от плоскости грани (0, 0, 1),
    // поэтому ndl = 1.0 везде, цвет равномерный 0.2 + 0.8*1.0 = 1.0 (255)
    let mut hal_flat = Hal::new(OutputMode::Silent, 8, 8);
    hal_flat.fill_all(0, 0, 0);
    сброс_света(&mut hal_flat);
    hal_flat.r3.light = [0.0, 0.0, 1.0];
    hal_flat.r3.ambient = 0.2;
    hal_flat.r3.diffuse = 0.8;
    hal_flat.r3.smooth = false;
    mesh_draw(
        &mut hal_flat,
        &pos,
        None,
        None,
        Some(&nrm),
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
    let left_flat = пиксель(&hal_flat, 0, 4);
    let right_flat = пиксель(&hal_flat, 7, 4);
    assert_eq!(left_flat[0], 255);
    assert_eq!(right_flat[0], 255);

    // 2. В режиме smooth: нормали интерполируются!
    // На левом краю нормаль близка к (-1, 0, 0) -> N·L ≈ 0 -> цвет ≈ ambient (0.2 -> ~51)
    // На правом краю нормаль близка к (0, 0, 1) -> N·L ≈ 1 -> цвет ≈ ambient + diffuse (1.0 -> 255)
    let mut hal_smooth = Hal::new(OutputMode::Silent, 8, 8);
    hal_smooth.fill_all(0, 0, 0);
    сброс_света(&mut hal_smooth);
    hal_smooth.r3.light = [0.0, 0.0, 1.0];
    hal_smooth.r3.ambient = 0.2;
    hal_smooth.r3.diffuse = 0.8;
    hal_smooth.r3.smooth = true;
    mesh_draw(
        &mut hal_smooth,
        &pos,
        None,
        None,
        Some(&nrm),
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
    let left_smooth = пиксель(&hal_smooth, 0, 4);
    let right_smooth = пиксель(&hal_smooth, 7, 4);
    assert!(
        left_smooth[0] < right_smooth[0],
        "левый край ({}) должен быть темнее правого ({})",
        left_smooth[0],
        right_smooth[0]
    );
    assert!(
        left_smooth[0] < 120,
        "левый край должен иметь низкую освещенность: {}",
        left_smooth[0]
    );
    assert_eq!(
        right_smooth[0], 255,
        "правый край освещен полностью: {}",
        right_smooth[0]
    );
}

#[test]
fn bilinear_and_mips() {
    // 1. Генерация пирамиды мипмапов (4x4 -> 2x2 -> 1x1)
    // Квадранты 2x2: Red, Green, Blue, White
    let mut px4x4 = vec![0u8; 4 * 4 * 4];
    for y in 0..4 {
        for x in 0..4 {
            let idx = (y * 4 + x) * 4;
            let color = match (x < 2, y < 2) {
                (true, true) => [255, 0, 0, 255],       // Верх-лево: Red
                (false, true) => [0, 255, 0, 255],      // Верх-право: Green
                (true, false) => [0, 0, 255, 255],      // Низ-лево: Blue
                (false, false) => [255, 255, 255, 255], // Низ-право: White
            };
            px4x4[idx..idx + 4].copy_from_slice(&color);
        }
    }
    let tex = Tex::new(4, 4, px4x4);
    assert_eq!(
        tex.mips.len(),
        3,
        "4x4 должна дать 3 уровня LOD (0: 4x4, 1: 2x2, 2: 1x1)"
    );
    assert_eq!((tex.mips[0].w, tex.mips[0].h), (4, 4));
    assert_eq!((tex.mips[1].w, tex.mips[1].h), (2, 2));
    assert_eq!((tex.mips[2].w, tex.mips[2].h), (1, 1));

    // LOD 1: усреднение 2x2 блоков сохраняет чистые цвета квадрантов
    let lod1_tl = &tex.mips[1].px[0..4];
    let lod1_tr = &tex.mips[1].px[4..8];
    let lod1_bl = &tex.mips[1].px[8..12];
    let lod1_br = &tex.mips[1].px[12..16];
    assert_eq!(lod1_tl, [255, 0, 0, 255], "LOD 1 верх-лево: Red");
    assert_eq!(lod1_tr, [0, 255, 0, 255], "LOD 1 верх-право: Green");
    assert_eq!(lod1_bl, [0, 0, 255, 255], "LOD 1 низ-лево: Blue");
    assert_eq!(lod1_br, [255, 255, 255, 255], "LOD 1 низ-право: White");

    // LOD 2: усреднение всех 4 цветов: (255+0+0+255)/4 = 128
    let lod2 = &tex.mips[2].px[0..4];
    assert_eq!(
        lod2,
        [128, 128, 128, 255],
        "LOD 2 1x1: среднее всех 4 квадрантов"
    );

    // Непрямоугольная текстура 8x2: 8x2 -> 4x1 -> 2x1 -> 1x1
    let tex_rec = Tex::new(8, 2, vec![100u8; 8 * 2 * 4]);
    assert_eq!(tex_rec.mips.len(), 4);
    assert_eq!((tex_rec.mips[1].w, tex_rec.mips[1].h), (4, 1));
    assert_eq!((tex_rec.mips[2].w, tex_rec.mips[2].h), (2, 1));
    assert_eq!((tex_rec.mips[3].w, tex_rec.mips[3].h), (1, 1));

    // 2. Билинейная интерполяция
    // 2x2 текстура: левый столбец Black, правый столбец White
    let bw_px = vec![
        0, 0, 0, 255, 255, 255, 255, 255, 0, 0, 0, 255, 255, 255, 255, 255,
    ];
    let bw_mip = crate::hal::MipLevel {
        w: 2,
        h: 2,
        px: bw_px,
    };

    // Крайние точки (u=0 и u=1)
    assert_eq!(sample_bilinear(&bw_mip, 0.0, 0.5), [0, 0, 0, 255]);
    assert_eq!(sample_bilinear(&bw_mip, 1.0, 0.5), [255, 255, 255, 255]);

    // Середина u=0.5 -> ровно 128 (взвешенное 50/50)
    let mid = sample_bilinear(&bw_mip, 0.5, 0.5);
    assert_eq!(mid[0], 128, "середина lerp: {}", mid[0]);

    // Четверти: u=0.25 -> ~64, u=0.75 -> ~191
    let q1 = sample_bilinear(&bw_mip, 0.25, 0.5);
    let q3 = sample_bilinear(&bw_mip, 0.75, 0.5);
    assert_eq!(q1[0], 64, "четверть 0.25: {}", q1[0]);
    assert_eq!(q3[0], 191, "четверть 0.75: {}", q3[0]);

    // Сравнение с Nearest: на 0.25 Nearest даёт ровно 0, на 0.75 ровно 255
    assert_eq!(sample_nearest(&bw_mip, 0.25, 0.5)[0], 0);
    assert_eq!(sample_nearest(&bw_mip, 0.75, 0.5)[0], 255);

    // 3. Интеграционный тест растра: ступенчатый Nearest vs гладкий Bilinear
    let uv = [0.0f32, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0];
    let mut hal_near = Hal::new(OutputMode::Silent, 8, 8);
    hal_near.tex.push(Tex::new(
        2,
        2,
        vec![
            0, 0, 0, 255, 255, 255, 255, 255, 0, 0, 0, 255, 255, 255, 255, 255,
        ],
    ));
    hal_near.fill_all(0, 0, 0);
    свет_плоский(&mut hal_near);
    hal_near.r3.tex_filter = TexFilter::Nearest;
    рисуем_квадрат(&mut hal_near, 1, 0.5, &uv);

    let mut hal_bili = Hal::new(OutputMode::Silent, 8, 8);
    hal_bili.tex.push(Tex::new(
        2,
        2,
        vec![
            0, 0, 0, 255, 255, 255, 255, 255, 0, 0, 0, 255, 255, 255, 255, 255,
        ],
    ));
    hal_bili.fill_all(0, 0, 0);
    свет_плоский(&mut hal_bili);
    hal_bili.r3.tex_filter = TexFilter::Bilinear;
    рисуем_квадрат(&mut hal_bili, 1, 0.5, &uv);

    // В Nearest: пиксель 3 черный (0), пиксель 4 белый (255) — резкая ступенька
    assert_eq!(пиксель(&hal_near, 0, 4)[0], 0);
    assert_eq!(пиксель(&hal_near, 3, 4)[0], 0);
    assert_eq!(пиксель(&hal_near, 4, 4)[0], 255);
    assert_eq!(пиксель(&hal_near, 7, 4)[0], 255);

    // В Bilinear: плавный градиент с переходом через середину
    let b0 = пиксель(&hal_bili, 0, 4)[0];
    let b3 = пиксель(&hal_bili, 3, 4)[0];
    let b4 = пиксель(&hal_bili, 4, 4)[0];
    let b7 = пиксель(&hal_bili, 7, 4)[0];
    assert!(b0 <= 20, "левый край bilinear: {}", b0);
    assert!(
        b3 > 50 && b3 < 150,
        "пиксель 3 в bilinear имеет промежуточный цвет: {}",
        b3
    );
    assert!(
        b4 > 120 && b4 < 200,
        "пиксель 4 в bilinear имеет промежуточный цвет: {}",
        b4
    );
    assert!(
        b0 < b3 && b3 < b4 && b4 < b7,
        "монотонный рост градиента: {} < {} < {} < {}",
        b0,
        b3,
        b4,
        b7
    );
    assert!(b7 >= 235, "правый край bilinear: {}", b7);
}

#[test]
fn shadow_and_point_lights() {
    // 1. Тест точечного источника света и затухания
    let pl = PointLight {
        pos: [0.0, 1.0, 0.0],
        color: [1.0, 0.5, 0.25],
        radius: 5.0,
        attenuation: 1.0,
    };
    let view = [0.0, 5.0, 5.0];
    let nrm = [0.0, 1.0, 0.0];

    // Точка прямо под источником: (0, 0, 0)
    let (diff0, _) = compute_point_lights(
        std::slice::from_ref(&pl),
        &view,
        0.0,
        0.0,
        0.0,
        &nrm,
        false,
        false,
        32.0,
        0.0,
    );
    assert!(
        diff0[0] > 0.5,
        "свет прямо под источником ярок: {}",
        diff0[0]
    );

    // Точка на расстоянии 3: свет заметно затухает
    let (diff3, _) = compute_point_lights(
        std::slice::from_ref(&pl),
        &view,
        3.0,
        0.0,
        0.0,
        &nrm,
        false,
        false,
        32.0,
        0.0,
    );
    assert!(
        diff3[0] > 0.0 && diff3[0] < diff0[0],
        "затухание с расстоянием: {} < {}",
        diff3[0],
        diff0[0]
    );

    // Точка на расстоянии 6 (> radius 5.0): свет строго равен 0
    let (diff6, _) = compute_point_lights(
        std::slice::from_ref(&pl),
        &view,
        6.0,
        0.0,
        0.0,
        &nrm,
        false,
        false,
        32.0,
        0.0,
    );
    assert_eq!(diff6, [0.0, 0.0, 0.0], "вне радиуса свет нулевой");

    // 2. Тест теневой карты (Shadow Mapping)
    let mut r3 = R3::new();
    let light_dir = [0.0, 1.0, 0.0]; // Свет светит строго сверху вниз
    let light_mat = compute_light_matrix(light_dir, [0.0, 0.0, 0.0], 10.0, 1.0, 100.0);

    shadow_begin(&mut r3, light_mat, 64, 64, Some(0.005));
    assert!(r3.shadow_pass);
    assert!(r3.shadow_enabled);
    assert_eq!(r3.shadow_map.len(), 64 * 64);

    // Рендерим кастер тени: квадрат размером 2x2 на высоте y = 5.0 над началом координат
    let caster_pos = [
        -1.0f32, 5.0, -1.0, 1.0, 5.0, -1.0, 1.0, 5.0, 1.0, -1.0, 5.0, 1.0,
    ];
    let caster_idx = [0.0f32, 1.0, 2.0, 0.0, 2.0, 3.0];
    let mut world = Vec::new();
    let mut world_nrm = Vec::new();
    let mut clip = Vec::new();
    let identity = [
        1.0f32, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ];
    transform_all(
        &caster_pos,
        None,
        &identity,
        &light_mat,
        &mut world,
        &mut world_nrm,
        &mut clip,
    );
    let drawn = raster_shadow_tris(&mut r3, &clip, &caster_idx).unwrap();
    assert!(drawn > 0, "треугольники кастера записаны в теневую карту");

    shadow_end(&mut r3);
    assert!(!r3.shadow_pass);
    assert!(r3.shadow_enabled);

    // Проверяем семплирование:
    // Точка (0, 0, 0) на земле находится строго под кастером -> коэффициент тени 0.0
    let s_shadow = shadow_sample(&r3, 0.0, 0.0, 0.0, 0.005);
    assert_eq!(s_shadow, 0.0, "точка под кастером полностью в тени");

    // Точка (8, 0, 8) далеко от кастера -> на прямом свету (1.0)
    let s_lit = shadow_sample(&r3, 8.0, 0.0, 8.0, 0.005);
    assert_eq!(s_lit, 1.0, "точка вдали от кастера полностью освещена");

    // Точка на границе кастера (~1.0, 0, 0) с PCF 2x2 даёт мягкий переход
    let s_edge = shadow_sample(&r3, 1.0, 0.0, 0.0, 0.005);
    assert!(
        (0.0..=1.0).contains(&s_edge),
        "мягкая граница тени PCF: {}",
        s_edge
    );
}

// ——— этап 9.2: инкрементальный сканлайн + LUT тумана ———

fn тест_вершина(sx: f32, sy: f32, iw: f32) -> Vtx {
    Vtx {
        sx,
        sy,
        sz: sx * 0.01,
        uq: sx * iw,
        vq: sy * iw,
        iw,
        wxq: sx * iw,
        wyq: 0.0,
        wzq: sy * iw,
        aq: iw,
        rq: sx * iw,
        gq: 0.5 * iw,
        bq: 0.0,
        nxq: 0.0,
        nyq: 0.0,
        nzq: iw,
    }
}

#[test]
fn сканлайн_спан_совпадает_с_перебором_bbox() {
    // прямоугольный, тупой, вертикальное ребро, «игла»; набор пикселей
    // row_span обязан совпадать с перебором edge-функций на пиксель
    let заготовки: Vec<[Vtx; 3]> = vec![
        [
            тест_вершина(2.0, 1.0, 1.0),
            тест_вершина(9.0, 3.0, 1.0),
            тест_вершина(4.0, 7.0, 1.0),
        ],
        [
            тест_вершина(0.0, 0.0, 1.0),
            тест_вершина(7.0, 6.5, 1.0),
            тест_вершина(1.0, 7.0, 1.0),
        ],
        [
            тест_вершина(3.0, 0.0, 1.0),
            тест_вершина(3.0, 7.0, 1.0),
            тест_вершина(8.0, 4.0, 1.0),
        ],
        [
            тест_вершина(1.0, 3.0, 1.0),
            тест_вершина(9.0, 3.4, 1.0),
            тест_вершина(5.0, 3.6, 1.0),
        ],
    ];
    for з in &заготовки {
        let mut tri = *з;
        // лицевая ориентация (area < 0) — конвенция opaque-прохода
        let area = edge(
            tri[0].sx, tri[0].sy, tri[1].sx, tri[1].sy, tri[2].sx, tri[2].sy,
        );
        if area > 0.0 {
            tri.swap(1, 2);
        }
        let (p0, p1, p2) = (&tri[0], &tri[1], &tri[2]);
        let ax = [p1.sy - p2.sy, p2.sy - p0.sy, p0.sy - p1.sy];
        let minx = (p0.sx.min(p1.sx).min(p2.sx).floor() as i64).max(0);
        let maxx = (p0.sx.max(p1.sx).max(p2.sx).ceil() as i64).min(7);
        let miny = (p0.sy.min(p1.sy).min(p2.sy).floor() as i64).max(0);
        let maxy = (p0.sy.max(p1.sy).max(p2.sy).ceil() as i64).min(7);
        for py in miny..=maxy {
            let fy = py as f32 + 0.5;
            let fx0 = minx as f32 + 0.5;
            let w0r = edge(p1.sx, p1.sy, p2.sx, p2.sy, fx0, fy);
            let w1r = edge(p2.sx, p2.sy, p0.sx, p0.sy, fx0, fy);
            let w2r = edge(p0.sx, p0.sy, p1.sx, p1.sy, fx0, fy);
            let (from, to) = row_span([w0r, w1r, w2r], ax, fx0, minx, maxx);
            for px in minx..=maxx {
                let fx = px as f32 + 0.5;
                let brute = edge(p1.sx, p1.sy, p2.sx, p2.sy, fx, fy) <= 0.0
                    && edge(p2.sx, p2.sy, p0.sx, p0.sy, fx, fy) <= 0.0
                    && edge(p0.sx, p0.sy, p1.sx, p1.sy, fx, fy) <= 0.0;
                assert_eq!(
                    from <= px && px <= to,
                    brute,
                    "строка {} пиксель {}: span ({},{})",
                    py,
                    px,
                    from,
                    to
                );
            }
        }
    }
}

#[test]
fn сумма_edge_функций_инвариантна_и_равна_площади() {
    // знаменатель барицентрика — константа треугольника: это даёт один
    // делитель на строку вместо одного на пиксель
    let tri = [
        тест_вершина(1.0, 2.0, 1.0),
        тест_вершина(8.0, 3.0, 1.0),
        тест_вершина(2.5, 7.0, 1.0),
    ];
    let (p0, p1, p2) = (&tri[0], &tri[1], &tri[2]);
    let area = edge(p0.sx, p0.sy, p1.sx, p1.sy, p2.sx, p2.sy);
    for (fx, fy) in [(3.5, 3.5), (0.5, 0.5), (9.5, 8.5), (5.2, 4.9)] {
        let s = edge(p1.sx, p1.sy, p2.sx, p2.sy, fx, fy)
            + edge(p2.sx, p2.sy, p0.sx, p0.sy, fx, fy)
            + edge(p0.sx, p0.sy, p1.sx, p1.sy, fx, fy);
        assert!(близко(s, area), "s={} area={}", s, area);
    }
}

#[test]
fn сканлайн_атрибуты_совпадают_с_барицентрикой() {
    // пошаговые числители N_q·inv_d обязаны давать те же значения, что
    // прямой барицентрик в каждом пикселе
    let mut tri = [
        тест_вершина(1.3, 0.7, 0.9),
        тест_вершина(7.9, 2.2, 1.1),
        тест_вершина(3.4, 6.8, 0.8),
    ];
    let area = edge(
        tri[0].sx, tri[0].sy, tri[1].sx, tri[1].sy, tri[2].sx, tri[2].sy,
    );
    if area > 0.0 {
        tri.swap(1, 2);
    }
    let (p0, p1, p2) = (&tri[0], &tri[1], &tri[2]);
    let ax = [p1.sy - p2.sy, p2.sy - p0.sy, p0.sy - p1.sy];
    let minx = (p0.sx.min(p1.sx).min(p2.sx).floor() as i64).max(0);
    let maxx = (p0.sx.max(p1.sx).max(p2.sx).ceil() as i64).min(7);
    let miny = (p0.sy.min(p1.sy).min(p2.sy).floor() as i64).max(0);
    let maxy = (p0.sy.max(p1.sy).max(p2.sy).ceil() as i64).min(7);
    const NA: usize = 11;
    let q: [[f32; 3]; NA] = [
        [p0.sz, p1.sz, p2.sz],
        [p0.iw, p1.iw, p2.iw],
        [p0.uq, p1.uq, p2.uq],
        [p0.vq, p1.vq, p2.vq],
        [p0.wxq, p1.wxq, p2.wxq],
        [p0.wyq, p1.wyq, p2.wyq],
        [p0.wzq, p1.wzq, p2.wzq],
        [p0.rq, p1.rq, p2.rq],
        [p0.gq, p1.gq, p2.gq],
        [p0.bq, p1.bq, p2.bq],
        [p0.aq, p1.aq, p2.aq],
    ];
    let mut проверено = 0;
    for py in miny..=maxy {
        let fy = py as f32 + 0.5;
        let fx0 = minx as f32 + 0.5;
        let w0r = edge(p1.sx, p1.sy, p2.sx, p2.sy, fx0, fy);
        let w1r = edge(p2.sx, p2.sy, p0.sx, p0.sy, fx0, fy);
        let w2r = edge(p0.sx, p0.sy, p1.sx, p1.sy, fx0, fy);
        let (from, to) = row_span([w0r, w1r, w2r], ax, fx0, minx, maxx);
        if from > to {
            continue;
        }
        let inv_d = 1.0 / (w0r + w1r + w2r);
        let wr = [w0r, w1r, w2r];
        let mut n = [0.0f32; NA];
        let mut dn = [0.0f32; NA];
        for a in 0..NA {
            n[a] = wr[0] * q[a][0] + wr[1] * q[a][1] + wr[2] * q[a][2];
            dn[a] = ax[0] * q[a][0] + ax[1] * q[a][1] + ax[2] * q[a][2];
        }
        let skip = (from - minx) as f32;
        for a in 0..NA {
            n[a] += dn[a] * skip;
        }
        for px in from..=to {
            let fx = px as f32 + 0.5;
            let w0 = edge(p1.sx, p1.sy, p2.sx, p2.sy, fx, fy);
            let w1 = edge(p2.sx, p2.sy, p0.sx, p0.sy, fx, fy);
            let w2 = edge(p0.sx, p0.sy, p1.sx, p1.sy, fx, fy);
            let sum = w0 + w1 + w2;
            for a in 0..NA {
                let прямой = (w0 * q[a][0] + w1 * q[a][1] + w2 * q[a][2]) / sum;
                assert!(
                    близко(n[a] * inv_d, прямой),
                    "атрибут {} пиксель ({},{}): {} против {}",
                    a,
                    px,
                    py,
                    n[a] * inv_d,
                    прямой
                );
            }
            проверено += 1;
            for a in 0..NA {
                n[a] += dn[a];
            }
        }
    }
    assert!(проверено > 0, "спан пуст — тест ничего не проверил");
}

#[test]
fn туман_lut_близок_к_точной_формуле() {
    let mut r3 = R3::new();
    r3.fog = [0.7, 0.8, 0.9, 0.05];
    fog_lut_sync(&mut r3);
    assert!(близко(r3.fog_lut[0], 0.0));
    // монотонность и насыщение к 1
    for i in 0..256 {
        assert!(r3.fog_lut[i + 1] >= r3.fog_lut[i]);
    }
    assert!(r3.fog_lut[256] > 0.99);
    // интерполяция между узлами неотличима от точной формулы (зона до
    // последнего узла; на финальном интервале LUT дотягивается до 1.0,
    // тогда как точная формула ползёт к 0.9975 — расхождение ≤ 0.25%
    // блендинга, невизуально)
    let mut max_err = 0.0f32;
    for k in 0..=238 {
        let depth = k as f32 * 0.5;
        let точный = fog_factor(0.05, depth);
        let через_lut = fog_lut_lookup(&r3.fog_lut, 0.05, depth);
        max_err = max_err.max((точный - через_lut).abs());
    }
    assert!(max_err < 1e-3, "max_err = {}", max_err);
    // хвост за span: LUT накрывает точную кривую и не выше 1
    for k in 239..=400 {
        let depth = k as f32 * 0.5;
        let точный = fog_factor(0.05, depth);
        let через_lut = fog_lut_lookup(&r3.fog_lut, 0.05, depth);
        assert!(
            через_lut >= точный - 1e-3 && через_lut <= 1.0,
            "depth {}: {} против {}",
            depth,
            через_lut,
            точный
        );
    }
    // density 0 — тумана нет
    assert!(близко(fog_lut_lookup(&r3.fog_lut, 0.0, 50.0), 0.0));
    // смена fog лениво перестраивает LUT (снимок-ключ)
    r3.fog = [0.1, 0.1, 0.1, 0.2];
    fog_lut_sync(&mut r3);
    // depth 25 — внутри span (depth 30 = ровно span: там LUT = 1.0,
    // точная формула 0.9975 — семантика насыщения, см. хвост выше)
    assert!(близко(
        fog_lut_lookup(&r3.fog_lut, 0.2, 25.0),
        fog_factor(0.2, 25.0)
    ));
}
