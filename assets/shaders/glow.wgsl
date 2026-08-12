const WIDTH: u32 = 128u;
const HEIGHT: u32 = 64u;

@group(0) @binding(0)
var<storage, read_write> pixels: array<u32>;

fn pack_white(alpha: f32) -> u32 {
    let a = u32(clamp(alpha, 0.0, 1.0) * 255.0 + 0.5);
    return 0x00ffffffu | (a << 24u);
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= WIDTH || id.y >= HEIGHT {
        return;
    }

    let uv = (vec2<f32>(id.xy) + vec2<f32>(0.5)) / vec2<f32>(f32(WIDTH), f32(HEIGHT));
    let point = abs(uv * 2.0 - vec2<f32>(1.0));
    let capsule = vec2<f32>(max(point.x - 0.34, 0.0) * 0.78, point.y);
    let distance = length(capsule);
    let halo = 1.0 - smoothstep(0.12, 1.0, distance);
    let core = 1.0 - smoothstep(0.04, 0.30, distance);
    let alpha = halo * halo * 0.48 + core * 0.16;

    pixels[id.y * WIDTH + id.x] = pack_white(alpha);
}
