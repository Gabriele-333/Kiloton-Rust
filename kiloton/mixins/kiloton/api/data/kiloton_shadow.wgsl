// Kiloton: the nuclear shadow (Java: `NuclearShadowEntity`), the figure of someone the flash killed, burnt
// onto the ground and stretched away from the fireball. A quad lying on the ground, the figure worked out in
// its own coordinates, hidden by what stands in front of it (the world's reversed depth). Premultiplied.

struct Shadow {
    view_proj: mat4x4<f32>,
    // xyz: the feet, from the camera; w: the direction away from the fireball (radians, atan2(dz, dx)).
    origin: vec4<f32>,
    // x: length of the figure, y: its darkness; zw: the depth texture's size.
    shape: vec4<f32>,
    // xy: the frame's size.
    screen: vec4<f32>,
};

@group(0) @binding(0) var<uniform> S: Shadow;
@group(0) @binding(1) var depth_tex: texture_depth_2d;

struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) local: vec2<f32>,
};

@vertex
fn vs(@builtin(vertex_index) i: u32) -> VOut {
    let len = S.shape.x;
    // Two triangles over the figure: along it from behind the feet to past the head, across a metre.
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-0.3, -0.5), vec2<f32>(len + 0.4, -0.5), vec2<f32>(len + 0.4, 0.5),
        vec2<f32>(-0.3, -0.5), vec2<f32>(len + 0.4, 0.5), vec2<f32>(-0.3, 0.5),
    );
    let c = corners[i];
    let dir = vec3<f32>(cos(S.origin.w), 0.0, sin(S.origin.w));
    let side = vec3<f32>(-dir.z, 0.0, dir.x);
    // A little over the ground: over a film of fallout dust too.
    let p = S.origin.xyz + dir * c.x + side * c.y + vec3<f32>(0.0, 0.08, 0.0);
    var o: VOut;
    o.pos = S.view_proj * vec4<f32>(p, 1.0);
    o.local = c;
    return o;
}

// 1 inside a rounded box (centre c, half sizes h), soft at the edge.
fn rbox(p: vec2<f32>, c: vec2<f32>, h: vec2<f32>) -> f32 {
    let q = abs(p - c) - h;
    let d = length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0);
    return 1.0 - smoothstep(-0.02, 0.03, d);
}

@fragment
fn fs(v: VOut) -> @location(0) vec4<f32> {
    // Hidden by whatever stands in front (reversed depth: more is nearer).
    let at = clamp(vec2<i32>(v.pos.xy * S.shape.zw / S.screen.xy), vec2<i32>(0), vec2<i32>(S.shape.zw) - vec2<i32>(1));
    let world = textureLoad(depth_tex, at, 0);
    if (world > v.pos.z + 0.00002) {
        discard;
    }
    let h = S.shape.x;
    let p = v.local;
    var a = 0.0;
    // Legs, body, arms, head: a standing player's figure laid down along the shadow.
    a = max(a, rbox(p, vec2<f32>(0.22 * h, 0.08), vec2<f32>(0.22 * h, 0.06)));
    a = max(a, rbox(p, vec2<f32>(0.22 * h, -0.08), vec2<f32>(0.22 * h, 0.06)));
    a = max(a, rbox(p, vec2<f32>(0.6 * h, 0.0), vec2<f32>(0.17 * h, 0.17)));
    a = max(a, rbox(p, vec2<f32>(0.6 * h, 0.23), vec2<f32>(0.15 * h, 0.055)));
    a = max(a, rbox(p, vec2<f32>(0.6 * h, -0.23), vec2<f32>(0.15 * h, 0.055)));
    a = max(a, rbox(p, vec2<f32>(0.88 * h, 0.0), vec2<f32>(0.1 * h, 0.13)));
    a *= S.shape.y;
    if (a < 0.01) {
        discard;
    }
    return vec4<f32>(vec3<f32>(0.04, 0.035, 0.03) * a, a);
}
