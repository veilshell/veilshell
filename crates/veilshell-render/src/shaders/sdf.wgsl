// VeilShell analytic SDF glassmorphic renderer (WGSL).
//
// Consumes the exact same `DrawList` primitives as `SdfSoftwareRenderer` in a
// single instanced render pass with premultiplied alpha blending so live
// Vulkan/GLES frames and CPU snapshots stay pixel-consistent.

struct ViewportUniforms {
    size: vec2<f32>,
    wallpaper_enabled: u32,
    _pad: u32,
};

@group(0) @binding(0)
var<uniform> u_viewport: ViewportUniforms;

@group(0) @binding(1)
var t_wallpaper: texture_2d<f32>;

@group(0) @binding(2)
var s_wallpaper: sampler;

struct PrimitiveInput {
    @location(0) rect: vec4<f32>,
    @location(1) color0: vec4<f32>,
    @location(2) color1: vec4<f32>,
    @location(3) params0: vec4<f32>,
    @location(4) params1: vec4<f32>,
    @location(5) kind_bits: vec4<u32>,
};

struct VertexOutput {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) @interpolate(flat) rect: vec4<f32>,
    @location(1) @interpolate(flat) color0: vec4<f32>,
    @location(2) @interpolate(flat) color1: vec4<f32>,
    @location(3) @interpolate(flat) params0: vec4<f32>,
    @location(4) @interpolate(flat) params1: vec4<f32>,
    @location(5) @interpolate(flat) kind_bits: vec4<u32>,
};

const KIND_BACKDROP: u32 = 0u;
const KIND_GLOW: u32 = 1u;
const KIND_RECT: u32 = 2u;
const KIND_GLYPH: u32 = 3u;

@vertex
fn vs_main(@builtin(vertex_index) vid: u32, inst: PrimitiveInput) -> VertexOutput {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 1.0),
    );
    let corner = corners[vid];
    let kind = inst.kind_bits.x;

    var quad_min = vec2<f32>(0.0, 0.0);
    var quad_size = u_viewport.size;

    if kind == KIND_GLOW {
        let r = inst.params0.z;
        quad_min = inst.params0.xy - vec2<f32>(r, r);
        quad_size = vec2<f32>(r * 2.0, r * 2.0);
    } else if kind == KIND_RECT {
        let pad = inst.params0.w;
        quad_min = inst.rect.xy - vec2<f32>(pad, pad);
        quad_size = inst.rect.zw + vec2<f32>(pad * 2.0, pad * 2.0);
    } else if kind == KIND_GLYPH {
        quad_min = inst.rect.xy;
        quad_size = inst.rect.zw;
    }

    let pos = quad_min + corner * quad_size;
    let safe_size = max(u_viewport.size, vec2<f32>(1.0, 1.0));
    let ndc = vec2<f32>(
        (pos.x / safe_size.x) * 2.0 - 1.0,
        1.0 - (pos.y / safe_size.y) * 2.0,
    );

    var out: VertexOutput;
    out.clip_pos = vec4<f32>(ndc, 0.0, 1.0);
    out.rect = inst.rect;
    out.color0 = inst.color0;
    out.color1 = inst.color1;
    out.params0 = inst.params0;
    out.params1 = inst.params1;
    out.kind_bits = inst.kind_bits;
    return out;
}

fn sdf_rounded_box(p: vec2<f32>, half_ext: vec2<f32>, r: f32) -> f32 {
    let q = abs(p) - (half_ext - vec2<f32>(r, r));
    if q.x > 0.0 && q.y > 0.0 {
        return length(q) - r;
    }
    return max(q.x, q.y) - r;
}

fn over_pm(dst: vec4<f32>, src_straight: vec4<f32>) -> vec4<f32> {
    let a = clamp(src_straight.a, 0.0, 1.0);
    if a <= 0.0005 {
        return dst;
    }
    let rgb = clamp(src_straight.rgb, vec3<f32>(0.0), vec3<f32>(1.0));
    let src_pm = vec4<f32>(rgb * a, a);
    return src_pm + dst * (1.0 - a);
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let frag = in.clip_pos.xy;
    let kind = in.kind_bits.x;

    if kind == KIND_BACKDROP {
        let denom = max(u_viewport.size.y - 1.0, 1.0);
        let frac = clamp(floor(frag.y) / denom, 0.0, 1.0);
        let grad = mix(in.color0, in.color1, frac);
        let a = clamp(grad.a, 0.0, 1.0);
        if u_viewport.wallpaper_enabled != 0u {
            let uv = clamp(frag / max(u_viewport.size, vec2<f32>(1.0, 1.0)), vec2<f32>(0.0), vec2<f32>(1.0));
            let wp = textureSampleLevel(t_wallpaper, s_wallpaper, uv, 0.0);
            let tinted = mix(wp.rgb, clamp(grad.rgb, vec3<f32>(0.0), vec3<f32>(1.0)), 0.56);
            return vec4<f32>(tinted * a, a);
        }
        return vec4<f32>(clamp(grad.rgb, vec3<f32>(0.0), vec3<f32>(1.0)) * a, a);
    }

    if kind == KIND_GLOW {
        let radius = in.params0.z;
        if radius <= 1.0 || in.color0.a <= 0.001 {
            discard;
        }
        let d = frag - in.params0.xy;
        let dist2 = dot(d, d) / (radius * radius);
        if dist2 >= 1.0 {
            discard;
        }
        let falloff = (1.0 - dist2) * (1.0 - dist2);
        let a = clamp(in.color0.a * falloff, 0.0, 1.0);
        return vec4<f32>(clamp(in.color0.rgb, vec3<f32>(0.0), vec3<f32>(1.0)) * a, a);
    }

    if kind == KIND_RECT {
        let half_ext = in.rect.zw * 0.5;
        let center = in.rect.xy + half_ext;
        let r = clamp(in.params0.x, 0.0, min(half_ext.x, half_ext.y));
        let border_width = in.params0.y;
        let top_highlight_alpha = in.params0.z;
        let shadow_radius = in.params1.x;
        let shadow_offset_y = in.params1.y;
        let shadow_alpha = in.params1.z;

        let rel = frag - center;
        let dist = sdf_rounded_box(rel, half_ext, r);
        var out = vec4<f32>(0.0, 0.0, 0.0, 0.0);

        if shadow_alpha > 0.001 && shadow_radius > 0.5 && dist > -1.0 {
            let sdist = sdf_rounded_box(rel - vec2<f32>(0.0, shadow_offset_y), half_ext, r);
            if sdist < shadow_radius {
                let norm = clamp(max(sdist, 0.0) / shadow_radius, 0.0, 1.0);
                let falloff = (1.0 - norm) * (1.0 - norm);
                let shadow_cov = clamp(dist + 0.5, 0.0, 1.0);
                out = over_pm(out, vec4<f32>(0.0, 0.0, 0.0, shadow_alpha * falloff * shadow_cov));
            }
        }

        let fill_cov = clamp(0.5 - dist, 0.0, 1.0);
        if fill_cov > 0.001 {
            out = over_pm(out, vec4<f32>(in.color0.rgb, in.color0.a * fill_cov));

            if top_highlight_alpha > 0.001 && in.rect.w > 0.0 {
                let norm_y = clamp((frag.y - in.rect.y) / in.rect.w, 0.0, 1.0);
                if norm_y < 0.35 {
                    let sheen = (1.0 - norm_y / 0.35) * top_highlight_alpha * fill_cov;
                    out = over_pm(out, vec4<f32>(1.0, 1.0, 1.0, sheen));
                }
            }

            if border_width > 0.01 && in.color1.a > 0.001 {
                let inner_cov = clamp(dist + border_width + 0.5, 0.0, 1.0);
                let border_cov = fill_cov * inner_cov;
                if border_cov > 0.001 {
                    out = over_pm(out, vec4<f32>(in.color1.rgb, in.color1.a * border_cov));
                }
            }
        }

        if out.a <= 0.0005 {
            discard;
        }
        return out;
    }

    // KIND_GLYPH: crisp 5x7 ASCII bitmap glyph packed into meta.y (rows 0..4) & meta.z (rows 4..7)
    let scale = max(in.params0.x, 1.0);
    let local = frag - in.rect.xy;
    let rx_i = i32(floor(local.x / scale));
    let ry_i = i32(floor(local.y / scale));
    if rx_i < 0 || rx_i >= 5 || ry_i < 0 || ry_i >= 7 {
        discard;
    }
    let rx = u32(rx_i);
    let ry = u32(ry_i);
    var row_mask: u32 = 0u;
    if ry < 4u {
        row_mask = (in.kind_bits.y >> (ry * 5u)) & 0x1Fu;
    } else {
        row_mask = (in.kind_bits.z >> ((ry - 4u) * 5u)) & 0x1Fu;
    }
    if ((row_mask >> (4u - rx)) & 1u) == 0u {
        discard;
    }
    let a = clamp(in.color0.a, 0.0, 1.0);
    if a <= 0.001 {
        discard;
    }
    return vec4<f32>(clamp(in.color0.rgb, vec3<f32>(0.0), vec3<f32>(1.0)) * a, a);
}
