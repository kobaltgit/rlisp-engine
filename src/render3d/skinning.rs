// src/render3d/skinning.rs — Деформация вершин по суставам (скелетная анимация).

/// Скиннинг по весам (этап 21): v' = Σ w_k · J_k · v, до 4 влияний на
/// вершину; веса нормируются по сумме (glTF требует суммарную 1, но файлы
/// бывают с весами вне [0,1]); jmat — финальные матрицы суставов
/// (мир узла × inverseBind), row-major 4×4, нижняя строка 0 0 0 1
pub(crate) fn skin_blend(
    pos: &[f32],
    joints: &[f32],
    weights: &[f32],
    jmat: &[f32],
) -> Result<Vec<f32>, String> {
    let nv = pos.len() / 3;
    if joints.len() != nv * 4 || weights.len() != nv * 4 {
        return Err(format!(
            "skin: joints/weights — по 4 на вершину, вершин {}",
            nv
        ));
    }
    let nj = jmat.len() / 16;
    if nj == 0 {
        return Err("skin: jmat пуст".into());
    }
    let mut out = Vec::with_capacity(nv * 3);
    for i in 0..nv {
        let x = pos[i * 3];
        let y = pos[i * 3 + 1];
        let z = pos[i * 3 + 2];
        let mut sx = 0.0f32;
        let mut sy = 0.0f32;
        let mut sz = 0.0f32;
        let mut wsum = 0.0f32;
        for k in 0..4 {
            let w = weights[i * 4 + k];
            if w <= 0.0 {
                continue;
            }
            let j = (joints[i * 4 + k] as usize).min(nj - 1);
            let m = &jmat[j * 16..j * 16 + 16];
            sx += w * (m[0] * x + m[1] * y + m[2] * z + m[3]);
            sy += w * (m[4] * x + m[5] * y + m[6] * z + m[7]);
            sz += w * (m[8] * x + m[9] * y + m[10] * z + m[11]);
            wsum += w;
        }
        let (ox, oy, oz) = if wsum > 1e-6 {
            (sx / wsum, sy / wsum, sz / wsum)
        } else {
            (x, y, z)
        };
        out.push(ox);
        out.push(oy);
        out.push(oz);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::skin_blend;

    #[test]
    fn identity_поза_не_меняет_вершины() {
        // единичные матрицы на все суставы, вес 1 на сустав 0
        let pos = vec![1.0, 2.0, 3.0, -4.0, 0.5, 7.0];
        let joints = vec![0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        let weights = vec![1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0];
        let mut jmat = Vec::new();
        for _ in 0..3 {
            jmat.extend([
                1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
            ]);
        }
        let out = skin_blend(&pos, &joints, &weights, &jmat).unwrap();
        for k in 0..pos.len() {
            assert!(
                (out[k] - pos[k]).abs() < 1e-5,
                "v{}: {} != {}",
                k,
                out[k],
                pos[k]
            );
        }
    }

    #[test]
    fn блендинг_двух_суставов_линейный() {
        // J0 = сдвиг (1,0,0), J1 = сдвиг (0,0,1); веса 0.5/0.5 → сдвиг (0.5,0,0.5)
        let pos = vec![2.0, 3.0, 4.0];
        let joints = vec![0.0, 1.0, 0.0, 0.0];
        let weights = vec![0.5, 0.5, 0.0, 0.0];
        let j0 = [
            1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ];
        let j1 = [
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0,
        ];
        let jmat: Vec<f32> = j0.iter().chain(j1.iter()).cloned().collect();
        let out = skin_blend(&pos, &joints, &weights, &jmat).unwrap();
        assert!((out[0] - 2.5).abs() < 1e-5, "x {}", out[0]);
        assert!((out[1] - 3.0).abs() < 1e-5, "y {}", out[1]);
        assert!((out[2] - 4.5).abs() < 1e-5, "z {}", out[2]);
    }

    #[test]
    fn веса_нормируются_по_сумме() {
        // веса 2/2 вместо 0.5/0.5 — результат тот же (нормировка по сумме)
        let pos = vec![2.0, 3.0, 4.0];
        let joints = vec![0.0, 1.0, 0.0, 0.0];
        let weights = vec![2.0, 2.0, 0.0, 0.0];
        let j0 = [
            1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ];
        let j1 = [
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0,
        ];
        let jmat: Vec<f32> = j0.iter().chain(j1.iter()).cloned().collect();
        let out = skin_blend(&pos, &joints, &weights, &jmat).unwrap();
        assert!((out[0] - 2.5).abs() < 1e-5, "x {}", out[0]);
        assert!((out[2] - 4.5).abs() < 1e-5, "z {}", out[2]);
    }

    #[test]
    fn индекс_за_границей_клампится() {
        // 3 сустава, индекс 99 → клампится к 2; нулевой вес не читает суставы
        let pos = vec![1.0, 1.0, 1.0];
        let joints = vec![99.0, 0.0, 0.0, 0.0];
        let weights = vec![1.0, 0.0, 0.0, 0.0];
        let mut jmat = Vec::new();
        // J2 — третья (индекс 2) = сдвиг (0, 5, 0); 99 клампится к 2
        jmat.extend([
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ]);
        jmat.extend([
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ]);
        jmat.extend([
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 5.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ]);
        let out = skin_blend(&pos, &joints, &weights, &jmat).unwrap();
        assert!((out[1] - 6.0).abs() < 1e-5, "y {}", out[1]);
    }

    #[test]
    fn детерминизм_побитовый() {
        let pos: Vec<f32> = (0..300).map(|i| (i % 37) as f32 - 10.0).collect();
        let joints: Vec<f32> = (0..400).map(|i| (i % 5) as f32).collect();
        let weights: Vec<f32> = (0..400).map(|i| ((i % 7) as f32) / 7.0).collect();
        let mut jmat = Vec::new();
        for j in 0..5 {
            let a = 0.3 + j as f32;
            jmat.extend([
                a.cos(),
                0.0,
                a.sin(),
                j as f32,
                0.0,
                1.0,
                0.0,
                0.0,
                -a.sin(),
                0.0,
                a.cos(),
                j as f32 * 0.5,
                0.0,
                0.0,
                0.0,
                1.0,
            ]);
        }
        let a = skin_blend(&pos, &joints, &weights, &jmat).unwrap();
        let b = skin_blend(&pos, &joints, &weights, &jmat).unwrap();
        assert_eq!(a, b, "блендинг недетерминирован");
    }

    #[test]
    fn ошибки_размеров() {
        assert!(skin_blend(&[0.0; 3], &[0.0; 3], &[0.0; 4], &[0.0; 16]).is_err());
        assert!(skin_blend(&[0.0; 3], &[0.0; 4], &[0.0; 4], &[0.0; 0]).is_err());
    }
}
