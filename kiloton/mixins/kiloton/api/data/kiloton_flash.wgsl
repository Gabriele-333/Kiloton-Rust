// Kiloton: the screen part of the flash (Java: `FlashOverlay`): the glare of each fireball where it is on
// screen, as big as the fireball looks (ViewerEffects.glare), and the after-image burnt into the eye, a
// purple-green blob that stays where the fireball was when it was seen. Soft round spots drawn here instead of
// the Java edition's glare.png and afterimage.png. Premultiplied (blend ONE, ONE_MINUS_SRC_ALPHA).

struct Flash {
    // xy: the frame's size in pixels; z: how many glares, w: how many after-images.
    screen: vec4<f32>,
    // Per glare: centre (pixels), size (pixels, the side of the Java blit), intensity.
    glare: array<vec4<f32>, 4>,
    // Per after-image: centre (pixels), size (pixels), alpha.
    after: array<vec4<f32>, 4>,
    // x: wobble of the after-image, pixels.
    misc: vec4<f32>,
};

@group(0) @binding(0) var<uniform> F: Flash;

struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs(@builtin(vertex_index) i: u32) -> VOut {
    let x = f32((i << 1u) & 2u);
    let y = f32(i & 2u);
    var o: VOut;
    o.pos = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
    o.uv = vec2<f32>(x, y);
    return o;
}

// A soft round spot of side `size`: 1 at the centre, 0 at the edge.
fn spot(p: vec2<f32>, c: vec2<f32>, size: f32) -> f32 {
    let x = clamp(length(p - c) / max(0.5 * size, 1.0), 0.0, 1.0);
    let k = 1.0 - x;
    return k * k * (0.35 + 0.65 * k);
}

fn over(dst: vec4<f32>, rgb: vec3<f32>, a: f32) -> vec4<f32> {
    return vec4<f32>(rgb * a, a) + dst * (1.0 - a);
}

@fragment
fn fs(v: VOut) -> @location(0) vec4<f32> {
    let p = v.pos.xy;
    var c = vec4<f32>(0.0);
    let ng = i32(F.screen.z);
    for (var i = 0; i < 4; i++) {
        if (i >= ng) {
            break;
        }
        let g = F.glare[i];
        c = over(c, vec3<f32>(1.0, 0.93, 0.8), clamp(g.w * spot(p, g.xy, g.z), 0.0, 1.0));
    }
    let na = i32(F.screen.w);
    let wob = F.misc.x;
    for (var i = 0; i < 4; i++) {
        if (i >= na) {
            break;
        }
        let a = F.after[i];
        c = over(c, vec3<f32>(0.55, 0.35, 0.75), clamp(a.w * spot(p, a.xy + vec2<f32>(wob, 0.0), a.z), 0.0, 1.0));
        c = over(c, vec3<f32>(0.35, 0.75, 0.45), clamp(0.6 * a.w * spot(p, a.xy - vec2<f32>(wob, 0.0), 0.55 * a.z), 0.0, 1.0));
    }
    if (c.a < 0.002) {
        discard;
    }
    return c;
}
