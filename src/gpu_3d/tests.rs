// src/gpu_3d/tests.rs — Unit-тесты для 3D GPU-конвейера.

use super::*;

#[test]
fn тест_размер_uniforms_raw_кратно_16_байт() {
    assert_eq!(std::mem::size_of::<Uniforms3DRaw>(), 208);
    assert_eq!(std::mem::size_of::<Uniforms3DRaw>() % 16, 0);
}

#[test]
fn тест_состояние_r3_конвертируется_в_uniforms() {
    let r3 = R3State {
        light: [0.1, 0.2, 0.3],
        ambient: 0.4,
        diffuse: 0.5,
        spec: 0.6,
        shininess: 32.0,
        view: [1.0, 2.0, 3.0],
        tiling: [2.0, 3.0],
        alpha_test: 0.5,
        fog: [0.7, 0.8, 0.9, 0.05],
        blend: 0.8,
        smooth: false,
        filter: crate::render3d::TexFilter::Nearest,
        mipmaps: false,
    };
    let raw = Uniforms3DRaw {
        model: [0.0; 16],
        mvp: [0.0; 16],
        light_dir: r3.light,
        ambient: r3.ambient,
        diffuse: r3.diffuse,
        spec: r3.spec,
        shininess: r3.shininess,
        alpha_test: r3.alpha_test,
        camera_pos: r3.view,
        blend_opacity: r3.blend,
        tiling: r3.tiling,
        fog_density: r3.fog[3],
        _pad0: 0.0,
        fog_color: [r3.fog[0], r3.fog[1], r3.fog[2]],
        _pad1: 0.0,
    };
    assert_eq!(raw.ambient, 0.4);
    assert_eq!(raw.shininess, 32.0);
    assert_eq!(raw.fog_density, 0.05);
}

#[cfg(feature = "wgpu")]
#[test]
fn тест_шейдер_3d_wgsl_содержит_все_точки_входа() {
    assert!(SHADER_3D.contains("fn vs_3d("));
    assert!(SHADER_3D.contains("fn vs_3d_instanced("));
    assert!(SHADER_3D.contains("fn fs_3d("));
    assert!(SHADER_3D.contains("fn fs_3d_tex("));
    assert!(SHADER_3D.contains("fn fs_3d_wire("));
}

#[cfg(feature = "wgpu")]
#[test]
fn тест_mesh_fingerprint_детектирует_изменения_вершин() {
    let mut pos = vec![0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0];
    let idx = vec![0.0, 1.0, 2.0];
    let fp1 = calc_mesh_fingerprint(&pos, &idx, None);
    let fp2 = calc_mesh_fingerprint(&pos, &idx, None);
    assert_eq!(fp1, fp2);

    // Симуляция динамической волны воды: меняется Y вершины в середине
    pos[4] = 9.99;
    let fp3 = calc_mesh_fingerprint(&pos, &idx, None);
    assert_ne!(fp1, fp3);
}
