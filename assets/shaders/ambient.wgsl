const WIDTH: u32 = 320u;
const HEIGHT: u32 = 160u;

@group(0) @binding(0)
var<storage, read_write> pixels: array<u32>;

fn pack_white(alpha: f32) -> u32 {
    let a = u32(clamp(alpha, 0.0, 1.0) * 255.0 + 0.5);
    return 0x00ffffffu | (a << 24u);
}

fn hash(point: vec2<u32>) -> f32 {
    var value = point.x * 1973u + point.y * 9277u + 0x68bc21ebu;
    value = (value ^ (value >> 15u)) * 2246822519u;
    value = (value ^ (value >> 13u)) * 3266489917u;
    return f32(value ^ (value >> 16u)) / 4294967295.0;
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= WIDTH || id.y >= HEIGHT {
        return;
    }

    let uv = (vec2<f32>(id.xy) + vec2<f32>(0.5)) / vec2<f32>(f32(WIDTH), f32(HEIGHT));
    let diagonal = uv.x * 0.72 + uv.y * 0.28;
    let beam_a = pow(max(0.0, 1.0 - abs(diagonal - 0.34) * 4.2), 2.4);
    let beam_b = pow(max(0.0, 1.0 - abs(diagonal - 0.76) * 5.0), 2.8);
    let curtain = 0.5 + 0.5 * sin((uv.x * 2.4 + uv.y * 0.8) * 6.2831853);
    let falloff = pow(1.0 - uv.y, 1.35);
    let grain = (hash(id.xy) - 0.5) * 0.012;
    let alpha = 0.014 + beam_a * 0.075 + beam_b * 0.048 + curtain * falloff * 0.014 + grain;

    pixels[id.y * WIDTH + id.x] = pack_white(alpha);
}
