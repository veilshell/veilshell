// Two-pass separable 13-tap Gaussian blur executed ONCE on wallpaper load to
// populate the persistent frosted glass wallpaper cache.

struct BlurUniforms {
    texel_dir: vec2<f32>,
    _pad: vec2<f32>,
};

@group(0) @binding(0)
var<uniform> u_blur: BlurUniforms;

@group(0) @binding(1)
var t_src: texture_2d<f32>;

@group(0) @binding(2)
var s_src: sampler;

struct VertexOutput {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vid: u32) -> VertexOutput {
    var positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    var uvs = array<vec2<f32>, 3>(
        vec2<f32>(0.0, 1.0),
        vec2<f32>(2.0, 1.0),
        vec2<f32>(0.0, -1.0),
    );
    var out: VertexOutput;
    out.clip_pos = vec4<f32>(positions[vid], 0.0, 1.0);
    out.uv = uvs[vid];
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let weights = array<f32, 7>(
        0.13298,
        0.12579,
        0.10648,
        0.08066,
        0.05467,
        0.03316,
        0.01799,
    );
    let step = u_blur.texel_dir;
    var acc = textureSampleLevel(t_src, s_src, clamp(in.uv, vec2<f32>(0.0), vec2<f32>(1.0)), 0.0) * weights[0];
    var total_w = weights[0];

    for (var i: i32 = 1; i <= 6; i = i + 1) {
        let offset = step * f32(i);
        let w = weights[i];
        let uv_pos = clamp(in.uv + offset, vec2<f32>(0.0), vec2<f32>(1.0));
        let uv_neg = clamp(in.uv - offset, vec2<f32>(0.0), vec2<f32>(1.0));
        acc = acc + (textureSampleLevel(t_src, s_src, uv_pos, 0.0) + textureSampleLevel(t_src, s_src, uv_neg, 0.0)) * w;
        total_w = total_w + 2.0 * w;
    }

    return vec4<f32>((acc / total_w).rgb, 1.0);
}
