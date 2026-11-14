const LINEAR: bool = false;
// Kiloton: lays the half-size cloud volume (kiloton_cloud.wgsl) over the frame. Four bilinear taps half a
// texel apart average each 3x3 block of the half-size image (weights 1-2-1), which cancels the dither of the
// march and smooths the edges. The first line is switched to true for a linear (sRGB or float) target.
@group(0) @binding(0) var half_tex: texture_2d<f32>;
@group(0) @binding(1) var half_samp: sampler;
struct VOut { @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32> };
@vertex fn vs(@builtin(vertex_index) i: u32) -> VOut {
    let x = f32((i << 1u) & 2u);
    let y = f32(i & 2u);
    var o: VOut;
    o.pos = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
    o.uv = vec2<f32>(x, y);
    return o;
}
@fragment fn fs(v: VOut) -> @location(0) vec4<f32> {
    let h = 0.5 / vec2<f32>(textureDimensions(half_tex));
    var c = 0.25 * (textureSample(half_tex, half_samp, v.uv + vec2<f32>(-h.x, -h.y)) + textureSample(half_tex, half_samp, v.uv + vec2<f32>(h.x, -h.y))
        + textureSample(half_tex, half_samp, v.uv + vec2<f32>(-h.x, h.y)) + textureSample(half_tex, half_samp, v.uv + vec2<f32>(h.x, h.y)));
    if (c.a < 0.002) {
        discard;
    }
    if (LINEAR) {
        // The march works in the display's gamma, as the Java edition's does; a linear target gets it back.
        let a = max(c.a, 1e-4);
        c = vec4<f32>(pow(c.rgb / a, vec3<f32>(2.2)) * a, c.a);
    }
    return c;
}
