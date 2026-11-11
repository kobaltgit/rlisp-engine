// src/gpu/wgpu_quad.rs — Шейдеры WGSL, исполнитель future и генерация квадов.

pub(crate) const SHADER: &str = r#"
struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) tint: vec3<f32>,
};

@vertex
fn vs_main(
    @location(0) pos: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) tint: vec3<f32>,
) -> VsOut {
    var out: VsOut;
    out.pos = vec4<f32>(pos, 0.0, 1.0);
    out.uv = uv;
    out.tint = tint;
    return out;
}

@group(0) @binding(0) var samp: sampler;
@group(0) @binding(1) var tex: texture_2d<f32>;

@fragment
fn fs_solid(@location(0) uv: vec2<f32>, @location(1) tint: vec3<f32>) -> @location(0) vec4<f32> {
    _ = uv; // phony-assignment: uv не участвует ('let _' невалиден в WGSL)
    return vec4<f32>(tint, 1.0);
}

@fragment
fn fs_tex(@location(0) uv: vec2<f32>, @location(1) tint: vec3<f32>) -> @location(0) vec4<f32> {
    let t = textureSample(tex, samp, uv);
    if (t.a == 0.0) { discard; } // как в программном blit: alpha=0 не пишем
    return vec4<f32>(t.rgb * tint, 1.0);
}
"#;

/// Мини-исполнитель фьючерсов (как pollster, но без зависимостей): wgpu на
/// нативе завершает фьючерсы внутренними потоками, достаточно спин-поллинга.
pub(crate) fn block_on<T>(fut: impl std::future::Future<Output = T>) -> T {
    use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

    fn noop(_: *const ()) {}
    fn cloner(p: *const ()) -> RawWaker {
        RawWaker::new(p, &VTABLE)
    }
    static VTABLE: RawWakerVTable = RawWakerVTable::new(cloner, noop, noop, noop);

    let waker = unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &VTABLE)) };
    let mut fut = std::pin::pin!(fut);
    let mut cx = Context::from_waker(&waker);
    loop {
        if let Poll::Ready(v) = fut.as_mut().poll(&mut cx) {
            return v;
        }
        std::thread::yield_now();
    }
}

pub(crate) struct Quad {
    // 6 вершин × 8 f32 (pos2 uv2 tint3 pad)
    pub(crate) data: [f32; 48],
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn quad_verts(
    x0: i64,
    y0: i64,
    dw: i64,
    dh: i64,
    w: usize,
    h: usize,
    uv0: [f32; 2],
    uv1: [f32; 2],
    tint: [f32; 3],
) -> Quad {
    // пиксели → NDC (y вниз, как в фреймбуфере)
    let ndc_x = |px: f32| px / w as f32 * 2.0 - 1.0;
    let ndc_y = |py: f32| 1.0 - py / h as f32 * 2.0;
    let (ax0, ay0) = (ndc_x(x0 as f32), ndc_y(y0 as f32));
    let (ax1, ay1) = (ndc_x((x0 + dw) as f32), ndc_y((y0 + dh) as f32));
    let [u0, v0] = uv0;
    let [u1, v1] = uv1;
    let mut v: Vec<f32> = Vec::with_capacity(48);
    let mut push = |x: f32, y: f32, u: f32, vv: f32| {
        v.extend_from_slice(&[x, y, u, vv, tint[0], tint[1], tint[2], 0.0]);
    };
    // два треугольника: TL TR BR | TL BR BL
    push(ax0, ay0, u0, v0);
    push(ax1, ay0, u1, v0);
    push(ax1, ay1, u1, v1);
    push(ax0, ay0, u0, v0);
    push(ax1, ay1, u1, v1);
    push(ax0, ay1, u0, v1);
    let mut data = [0f32; 48];
    data.copy_from_slice(&v);
    Quad { data }
}

pub(crate) fn bytemuck_bytes(data: &[f32]) -> &[u8] {
    unsafe { std::slice::from_raw_parts(data.as_ptr() as *const u8, data.len() * 4) }
}
