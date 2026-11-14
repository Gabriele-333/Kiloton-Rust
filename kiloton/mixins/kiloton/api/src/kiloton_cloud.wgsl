// Kiloton: the mushroom cloud as one volume, ray marched (WGSL port of kiloton_cloud_volume.glsl and
// cloud_volume.fsh of the Java edition, same shapes, motion, light and haze). One full-screen
// triangle per cloud into a half-size target, farthest first; the composite pass below lays it over
// the frame. Premultiplied (blend ONE, ONE_MINUS_SRC_ALPHA).
//
//   U.p[0..11]  the cloud at this instant, from MushroomCloud::volume (kiloton-core):
//     p0  camera relative to ground zero (xyz), seconds since the burst
//     p1  height of the centre of the head, radius of the head, half height of the head, radius of the stem
//     p2  top of the stem, roll of the vortex (radians), heat of the fireball (1 .. 0), fade of the whole cloud
//     p3  unit vector to the sun, daylight (0 night .. 1 day)
//     p4  sunlight colour, how formed the stem is
//     p5  skylight colour, haze visibility (blocks)
//     p6  haze colour, haze brightness
//     p7  colour of the head (red-brown while hot, then white), how formed the head is
//     p8  colour of the glowing air, light of the fireball still glowing inside the head
//     p9  centre height and radius of the fireball, how much of it still shows, fireball radius at full size
//     p10 radius and height of the base surge, how much of it shows, how far the afterwinds pulled it back in
//     p11 noise offset of this cloud, quality, rise speed of the stem (blocks/s), brightness of the glow
//   U.p[12]     z: for a cloud past the drawn terrain the tangent of where that terrain ends, else -10
//               (the skyline texture of the Java edition is not used here: y is 0)
//   U.p[13]     the Wilson cloud: radius of the shell, half thickness, phase (0 .. 2), amount; 0 when none
//   U.inv       clip to camera-relative world (reversed depth)
//   U.sizes     the depth texture's size (xy)

struct Cloud {
    p: array<vec4<f32>, 14>,
    inv: mat4x4<f32>,
    sizes: vec4<f32>,
};

@group(0) @binding(0) var<uniform> U: Cloud;
@group(0) @binding(1) var noise_tex: texture_2d<f32>;
@group(0) @binding(2) var noise_samp: sampler;
@group(0) @binding(3) var depth_tex: texture_depth_2d;

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

// ---------------------------------------------------------------------------------------------------------------
// Noise: a 64^3 tiling noise in 8 x 8 slices of 66 x 66 texels (one texel of border each side).

fn kn(p_in: vec3<f32>) -> vec4<f32> {
    let p = fract(p_in) * 64.0;
    let z = p.z - 0.5;
    let z0 = floor(z);
    let fz = z - z0;
    let k0 = (z0 + 64.0) % 64.0;
    let k1 = (z0 + 65.0) % 64.0;
    let xy = p.xy + 1.0;
    let a = vec2<f32>(k0 % 8.0, floor(k0 / 8.0)) * 66.0 + xy;
    let b = vec2<f32>(k1 % 8.0, floor(k1 / 8.0)) * 66.0 + xy;
    return mix(textureSampleLevel(noise_tex, noise_samp, a / 528.0, 0.0), textureSampleLevel(noise_tex, noise_samp, b / 528.0, 0.0), fz);
}

// ---------------------------------------------------------------------------------------------------------------
// Shape (blocks, ground zero at the origin, y up)

fn kSdEll2(p: vec2<f32>, r: vec2<f32>) -> f32 {
    let k0 = length(p / r);
    let k1 = length(p / (r * r));
    if (k1 > 1e-6) {
        return k0 * (k0 - 1.0) / k1;
    }
    return -min(r.x, r.y);
}

fn kSdEll3(p: vec3<f32>, r: vec3<f32>) -> f32 {
    let k0 = length(p / r);
    let k1 = length(p / (r * r));
    if (k1 > 1e-6) {
        return k0 * (k0 - 1.0) / k1;
    }
    return -min(r.x, min(r.y, r.z));
}

fn kSmin(a: f32, b: f32, k: f32) -> f32 {
    let h = clamp(0.5 + 0.5 * (b - a) / k, 0.0, 1.0);
    return mix(b, a, h) - k * h * (1.0 - h);
}

// Per pixel constants.
var<private> kHc: f32;
var<private> kRc: f32;
var<private> kChh: f32;
var<private> kRs: f32;
var<private> kStemTop: f32;
var<private> kSpin: f32;
var<private> kHeat: f32;
var<private> kR: f32;
var<private> kRm: f32;
var<private> kYf: f32;
var<private> kFireR: f32;
var<private> kFireOn: f32;
var<private> kSkirtR: f32;
var<private> kSkirtH: f32;
var<private> kSkirtOn: f32;
// The Wilson cloud: shell radius, half thickness, envelope, amount; and how much of the density at the point is it.
var<private> kWr: f32;
var<private> kWt: f32;
var<private> kWe: f32;
var<private> kWa: f32;
var<private> kWil: f32;
// Distance to the cloud itself (head, stem, surge, fireball), the Wilson shell left out: set by kDensity.
var<private> kDistBody: f32;
// The eye, relative to ground zero.
var<private> kEye: vec3<f32>;

fn kStemRadius(y: f32) -> f32 {
    let zf = clamp(y / max(kStemTop, 1.0), 0.0, 1.0);
    return kRs * (1.0 + exp(-zf * 7.0)) * (1.0 + 0.6 * smoothstep(0.8, 1.0, zf));
}

fn kHeadShape(p: vec3<f32>) -> f32 {
    let q = vec2<f32>(length(p.xz) - kR, p.y - kHc);
    let torus = kSdEll2(q, vec2<f32>(kRm, kChh));
    var dome = kSdEll3(p - vec3<f32>(0.0, kHc + 0.12 * kChh, 0.0), vec3<f32>(0.9 * kRc, 1.05 * kChh, 0.9 * kRc));
    dome = max(dome, kHc - 0.25 * kChh - p.y);
    return kSmin(torus, dome, 0.18 * kRc);
}

// Ground zero is the burst point's ground; the land round it can be lower (a hill, a coast): the stem and the base
// surge go on down a little, and the terrain in front hides what is under it.
const K_BELOW: f32 = 48.0;

fn kStemShape(p: vec3<f32>) -> f32 {
    var d = (length(p.xz) - kStemRadius(max(p.y, 0.0))) * 0.8;
    d = max(d, -p.y - K_BELOW);
    return max(d, p.y - (kStemTop + 0.7 * kChh));
}

// The base surge: a low layer of dust, thicker where it rolls at the front.
fn kSkirtShape(p: vec3<f32>) -> f32 {
    let r = length(p.xz);
    let rr = r / max(kSkirtR, 1.0);
    let hgt = kSkirtH * (0.3 + 0.7 * smoothstep(0.25, 0.85, rr)) * sqrt(max(1.0 - pow(rr, 6.0), 0.0));
    let d = max(p.y - hgt, (r - kSkirtR) * 0.7);
    return max(d, -p.y - K_BELOW);
}

// The Wilson cloud: a dome-shaped shell round ground zero just behind the shock front.
fn kWilsonShape(p: vec3<f32>) -> f32 {
    return max(abs(length(p) - kWr) - kWt, -p.y - K_BELOW);
}

fn kFireShape(p: vec3<f32>) -> f32 {
    let q = p - vec3<f32>(0.0, kYf, 0.0);
    return max(length(q) - kFireR, -p.y - 1.0);
}

// ---------------------------------------------------------------------------------------------------------------
// Motion of the material: the head rolls round its ring (up the inside, out over the top, down the outside), the stem
// rises, the base surge spreads. Drawn as two layers restarted in turn and blended (flow noise).

fn kRoll(p: vec3<f32>, ang: f32) -> vec3<f32> {
    let rxz = length(p.xz) + 1e-3;
    let dir = p.xz / rxz;
    var u = vec2<f32>(rxz - kR, p.y - kHc);
    let s = u / vec2<f32>(1.6 * kRm, 1.6 * kChh);
    let w = exp(-dot(s, s)) * smoothstep(0.0, 0.6 * kR, rxz);
    let a = ang * w;
    let c = cos(a);
    let sn = sin(a);
    u = vec2<f32>(c * u.x - sn * u.y, sn * u.x + c * u.y);
    return vec3<f32>(dir.x * (kR + u.x), kHc + u.y, dir.y * (kR + u.x));
}

// Billows of the head: displacement of its surface, blocks (positive = outwards).
fn kHeadBillows(p: vec3<f32>, seed: f32, fine: bool) -> f32 {
    let CYC = 1.4;
    let ph = kSpin / CYC;
    let f0 = fract(ph);
    let f1 = fract(ph + 0.5);
    let w0 = 1.0 - abs(2.0 * f0 - 1.0);
    let L = 1.05 * kRc;
    // A slow turn round the axis as well, the same for the whole head (no shear).
    let az = 0.004 * U.p[0].w;
    let rot = mat2x2<f32>(vec2<f32>(cos(az), sin(az)), vec2<f32>(-sin(az), cos(az)));
    var q = p;
    let qxz = rot * q.xz;
    q = vec3<f32>(qxz.x, q.y, qxz.y);
    let m0 = kRoll(q, f0 * CYC) / L + vec3<f32>(0.37, 0.11, 0.73) * floor(ph) + seed;
    let m1 = kRoll(q, f1 * CYC) / L + vec3<f32>(0.61, 0.29, 0.17) * floor(ph + 0.5) + seed + 0.5;
    let big = mix(kn(m1).r, kn(m0).r, w0);
    var d = kRc * 0.2 * (big - 0.45);
    if (fine) {
        let small = mix(kn(m1 * 2.6).g, kn(m0 * 2.6).g, w0);
        d += kRc * 0.085 * (small - 0.5);
    }
    return d;
}

fn kStemBillows(p: vec3<f32>, seed: f32, fine: bool) -> f32 {
    let L = 3.2 * kRs;
    let az = 0.05 * U.p[0].w;
    let q = vec3<f32>(cos(az) * p.x - sin(az) * p.z, p.y - U.p[11].z * U.p[0].w, sin(az) * p.x + cos(az) * p.z);
    let m = q / L + seed + 0.21;
    var d = kRs * 0.45 * (kn(m).r - 0.45);
    if (fine) {
        d += kRs * 0.16 * (kn(m * 2.5).g - 0.5);
    }
    return d;
}

fn kSkirtBillows(p: vec3<f32>, seed: f32) -> f32 {
    let L = 0.35 * max(kSkirtR, 10.0);
    let m = vec3<f32>(p.x, p.y * 2.0, p.z) / L + seed + 0.43;
    return kSkirtH * (0.9 * (kn(m).r - 0.45) + 0.4 * (kn(m * 2.7).g - 0.5));
}

struct Sample {
    den: f32,
    mat: vec4<f32>,
    dist: f32,
};

// Density (0 .. 1) at p, its material (x head, y stem, z base surge, w fireball) and the signed distance used to step.
fn kDensity(p: vec3<f32>, fine: bool) -> Sample {
    var out: Sample;
    let seed = U.p[11].x;
    let dh = kHeadShape(p);
    let ds = kStemShape(p);
    var dk = 1e9;
    if (kSkirtR > 1.0) {
        dk = kSkirtShape(p);
    }
    var df = 1e9;
    if (kFireOn > 0.001) {
        df = kFireShape(p);
    }
    var dw = 1e9;
    if (kWa > 0.0) {
        dw = kWilsonShape(p);
    }
    kWil = 0.0;
    let margH = 0.17 * kRc;
    let margS = 0.5 * kRs;
    let margK = 0.9 * kSkirtH;
    var d = 1e9;
    var wH = 0.0;
    var wS = 0.0;
    var wK = 0.0;
    var wF = 0.0;
    // Head and stem are one body: the stem flares into the head.
    let hs = kSmin(dh, ds, 0.5 * kRs);
    kDistBody = min(hs - max(margH, margS), min(dk - margK, df));
    let near = min(kDistBody, dw);
    if (near > 0.0) {
        out.den = 0.0;
        out.mat = vec4<f32>(0.0);
        out.dist = near;
        return out;
    }
    var bh = 0.0;
    if (dh < margH) {
        bh = kHeadBillows(p, seed, fine);
    }
    var bs = 0.0;
    if (ds < margS) {
        bs = kStemBillows(p, seed, fine);
    }
    let kHS = clamp(0.5 + 0.5 * (ds - dh) / (0.6 * kRs), 0.0, 1.0);
    let body = hs - mix(bs, bh, kHS);
    let bodyDen = clamp(-body / (0.022 * kRc), 0.0, 1.0);
    let headOn = U.p[7].w * U.p[2].w;
    let stemOn = U.p[4].w * U.p[2].w;
    let on = mix(stemOn, headOn, kHS);
    var den = bodyDen * on;
    wH = bodyDen * kHS * headOn;
    wS = bodyDen * (1.0 - kHS) * stemOn;
    d = body;
    if (dk < margK) {
        let sk = dk - kSkirtBillows(p, seed);
        var skDen = clamp(-sk / (0.6 * kSkirtH), 0.0, 1.0) * kSkirtOn * 0.45;
        // Seen from inside, the surge thins out round the eye (the dust in the air there is the haze round the
        // viewer, drawn apart); from outside nothing changes.
        if (fine) {
            skDen *= smoothstep(0.5 * kSkirtH, 4.0 * kSkirtH, length(p - kEye));
        }
        wK = skDen;
        den = max(den, skDen);
        d = min(d, sk);
    } else {
        d = min(d, dk - margK);
    }
    if (df < 0.15 * kFireR + 1.0) {
        let fb = df - 0.12 * kFireR * (kn((p - vec3<f32>(0.0, kYf, 0.0)) / (1.2 * kFireR) + vec3<f32>(0.0, -0.05 * U.p[0].w, 0.0) + seed).r - 0.5);
        let fDen = clamp(-fb / (0.05 * kFireR), 0.0, 1.0) * kFireOn;
        wF = fDen;
        den = max(den, fDen);
        d = min(d, fb);
    } else {
        d = min(d, df);
    }
    let sum = wH + wS + wK + wF;
    var mat = vec4<f32>(0.0);
    if (sum > 1e-5) {
        mat = vec4<f32>(wH, wS, wK, wF) / sum;
    }
    // The Wilson shell: thin and milky, lumpy, breaking up into patches as it evaporates.
    if (dw < 0.0) {
        let m = p / max(0.3 * kWr, 20.0) + seed + 0.77;
        let n = kn(m).b * 0.65 + kn(m * 2.9).r * 0.35;
        let edge = clamp(-dw / (0.6 * kWt), 0.0, 1.0);
        let form = clamp(kWe, 0.0, 1.0);
        let gone = clamp(kWe - 1.0, 0.0, 1.0);
        let wDen = edge * 0.7 * min(kWa, 2.0) * kRc / (68.0 * kWt) * form * (0.6 + 0.8 * n) * smoothstep(gone - 0.1, gone + 0.25, n);
        if (wDen > den) {
            kWil = 1.0 - den / max(wDen, 1e-5);
        }
        den = max(den, wDen);
    }
    kDistBody = d;
    d = min(d, dw);
    out.den = den;
    out.mat = mat;
    out.dist = d;
    return out;
}

// Coarse density for the shadow rays: shape and big billows only.
fn kShadowDensity(p: vec3<f32>) -> f32 {
    return kDensity(p, false).den;
}

// Coarser still, for the far shadow steps: the shapes alone, no noise.
fn kShapeDensity(p: vec3<f32>) -> f32 {
    let hs = kSmin(kHeadShape(p), kStemShape(p), 0.5 * kRs);
    var d = clamp(-(hs + 0.05 * kRc) / (0.06 * kRc), 0.0, 1.0) * U.p[2].w;
    if (kSkirtR > 1.0) {
        d = max(d, clamp(-kSkirtShape(p) / (0.6 * kSkirtH), 0.0, 1.0) * kSkirtOn * 0.45);
    }
    return d;
}

// Length of the ray from p along dir inside the ellipsoid of radii r centred at c.
fn kChordEll(p: vec3<f32>, dir: vec3<f32>, c: vec3<f32>, r: vec3<f32>) -> f32 {
    let q = (p - c) / r;
    let e = dir / r;
    let a = dot(e, e);
    let b = dot(q, e);
    let cc = dot(q, q) - 1.0;
    let disc = b * b - a * cc;
    if (disc <= 0.0) {
        return 0.0;
    }
    let sq = sqrt(disc);
    let t1 = (-b + sq) / a;
    if (t1 <= 0.0) {
        return 0.0;
    }
    return t1 - max((-b - sq) / a, 0.0);
}

// ---------------------------------------------------------------------------------------------------------------

fn kilotonCloud(ro: vec3<f32>, rd: vec3<f32>, tMax: f32, fragCoord: vec2<f32>) -> vec4<f32> {
    kHc = U.p[1].x;
    kRc = max(U.p[1].y, 1.0);
    kChh = max(U.p[1].z, 1.0);
    kRs = max(U.p[1].w, 0.5);
    kStemTop = U.p[2].x;
    kSpin = U.p[2].y;
    kHeat = U.p[2].z;
    kR = 0.58 * kRc;
    kRm = 0.42 * kRc;
    kYf = U.p[9].x;
    kFireR = max(U.p[9].y, 0.5);
    kFireOn = U.p[9].z;
    kSkirtR = U.p[10].x;
    kSkirtH = max(U.p[10].y, 1.0);
    kSkirtOn = U.p[10].z;
    // The afterwinds pull part of the base surge back to the stem: the ring shrinks a little and thickens.
    kSkirtR *= 1.0 - 0.3 * U.p[10].w;
    kSkirtH *= 1.0 + 0.5 * U.p[10].w;
    kWr = U.p[13].x;
    kWt = max(U.p[13].y, 1.0);
    kWe = U.p[13].z;
    kWa = U.p[13].w;
    kEye = ro;

    // Bounding cylinder.
    var top = max(kHc + 1.35 * kChh + 0.2 * kRc, kYf + 1.3 * kFireR);
    var rad = max(max(1.3 * kRc, 1.1 * kSkirtR + kSkirtH), 1.3 * kFireR);
    if (kWa > 0.0) {
        top = max(top, kWr + kWt);
        rad = max(rad, kWr + kWt);
    }
    var t0 = 0.0;
    var t1 = tMax;
    {
        let a = dot(rd.xz, rd.xz);
        let b = dot(ro.xz, rd.xz);
        let c = dot(ro.xz, ro.xz) - rad * rad;
        if (a > 1e-9) {
            let disc = b * b - a * c;
            if (disc <= 0.0) {
                return vec4<f32>(0.0);
            }
            let sq = sqrt(disc);
            t0 = max(t0, (-b - sq) / a);
            t1 = min(t1, (-b + sq) / a);
        } else if (c > 0.0) {
            return vec4<f32>(0.0);
        }
        if (abs(rd.y) > 1e-9) {
            let ya = (-K_BELOW - 2.0 - ro.y) / rd.y;
            let yb = (top - ro.y) / rd.y;
            t0 = max(t0, min(ya, yb));
            t1 = min(t1, max(ya, yb));
        } else if (ro.y < -K_BELOW - 2.0 || ro.y > top) {
            return vec4<f32>(0.0);
        }
    }
    if (t1 <= t0) {
        return vec4<f32>(0.0);
    }

    let sunDir = U.p[3].xyz;
    let sunCol = U.p[4].rgb;
    let skyCol = U.p[5].rgb;
    let cosT = dot(rd, sunDir);
    // Forward scattering: thin edges in front of the sun light up (silver lining).
    let silver = pow(max(cosT, 0.0), 10.0);
    let hot = U.p[8].rgb;
    let fireLight = U.p[8].w;
    let heat = kHeat;

    // Static dither of the start (interleaved gradient noise): no banding, no crawling.
    let ign = fract(52.9829189 * fract(dot(fragCoord, vec2<f32>(0.06711056, 0.00583715))));
    let stepIn = max(0.016 * kRc, 0.0011 * t0);
    var t = t0 + ign * stepIn;
    var T = 1.0;
    var col = vec3<f32>(0.0);
    var dSum = 0.0;
    var wSum = 0.0;
    var hSum = 0.0;
    let sigma = 34.0 / kRc;
    let maxSteps = i32(160.0 * U.p[11].y);
    var dense = 0;
    for (var i = 0; i < 400; i++) {
        if (i >= maxSteps || t > t1 || T < 0.015) {
            break;
        }
        let p = ro + rd * t;
        let s = kDensity(p, true);
        let den = s.den;
        let mat = s.mat;
        let dist = s.dist;
        var dt = max(0.016 * kRc, 0.0011 * t);
        // Through the thin Wilson shell, step by a quarter of its half thickness, never past the cloud's own surface.
        var shellStep = 0.0;
        if (kWa > 0.0 && abs(length(p) - kWr) < kWt) {
            shellStep = min(0.25 * kWt, max(kDistBody * 0.75, 0.5 * dt));
        }
        if (den < 0.002) {
            t += max(max(dist * 0.75, 0.5 * dt), shellStep);
            continue;
        }
        // Finer steps while entering the cloud (the edge is where the eye looks), longer ones deeper in.
        if (T > 0.85) {
            dt *= 0.5;
        } else {
            dt *= min(1.0 + 0.2 * f32(dense), 3.0);
        }
        if (kWil > 0.5) {
            dt = max(dt, shellStep);
        }
        dense++;
        let tr = exp(-den * sigma * dt);
        let wgt = T * (1.0 - tr);
        if (wgt < 0.003) {
            T *= tr;
            t += dt;
            continue;
        }
        // Sunlight: a few steps towards the sun (the near ones with the billows, the far ones on the shapes alone),
        // then the rest of the head as an ellipsoid (it shades the stem and the base).
        var tau = 0.0;
        var prev = 0.0;
        for (var k = 1; k <= 4; k++) {
            let sk = kRc * 0.45 * pow(f32(k) / 4.0, 1.8);
            let q = p + sunDir * sk;
            var dq = 0.0;
            if (k <= 2) {
                dq = kShadowDensity(q);
            } else {
                dq = kShapeDensity(q);
            }
            tau += dq * (sk - prev);
            prev = sk;
        }
        let far = kChordEll(p + sunDir * prev, sunDir, vec3<f32>(0.0, kHc + 0.1 * kChh, 0.0), vec3<f32>(0.95 * kRc, 1.05 * kChh, 0.95 * kRc));
        // The dust layer is thin and wide: its own shadow along it would come out as dark streaks.
        tau = (tau * 9.0 / kRc + far * 3.0 / kRc) * (1.0 - 0.6 * mat.z);
        let sunT = exp(-tau) + 0.1 * exp(-0.15 * tau);
        // Skylight: from above; the underside of the head and the depth of a crevice see less of it.
        let hFrac = mix(0.62, smoothstep(kHc - 1.1 * kChh, kHc + 1.1 * kChh, p.y), mat.x);
        let ao = clamp(-dist / (0.1 * kRc), 0.0, 1.0);
        let sky = (0.55 + 0.45 * hFrac) * (1.0 - 0.4 * ao);
        let bounce = 0.12 * (1.0 - hFrac) * U.p[3].w;
        // Colour of what is there.
        let zf = clamp(p.y / max(kStemTop, 1.0), 0.0, 1.0);
        let stemAlb = mix(vec3<f32>(0.50, 0.44, 0.38), vec3<f32>(0.66, 0.62, 0.58), zf);
        var alb = U.p[7].rgb * mat.x + stemAlb * mat.y + vec3<f32>(0.52, 0.47, 0.41) * mat.z + vec3<f32>(0.25, 0.18, 0.14) * mat.w;
        // Condensed water is white and scatters forward strongly; it does not glow.
        alb = mix(alb, vec3<f32>(0.94, 0.95, 0.97), kWil);
        let light = skyCol * 0.62 * sky + sunCol * (2.1 * sunT * (1.0 + 0.8 * silver * exp(-0.7 * tau)) + bounce);
        // Glow: the fireball, the core of the head while it is hot, the top of the stem, the surge near the middle.
        let inner = clamp(-dist / (0.25 * kRc), 0.0, 1.0);
        let glow = mat.w * 4.0 * pow(heat, 0.7)
            + mat.x * (3.0 * pow(heat, 0.8) * (0.5 + 0.5 * inner) + 0.6 * pow(heat, 0.5) * inner)
            + mat.y * 0.6 * pow(heat, 0.9) * zf * zf * zf
            + mat.z * 0.9 * heat * exp(-length(p.xz) / max(U.p[9].w, 1.0));
        let under = alb * fireLight * (0.3 + 0.7 * (1.0 - hFrac)) * vec3<f32>(1.1, 0.55, 0.22) * (mat.x + 0.5 * mat.y);
        let S = alb * light + (hot * glow * U.p[11].w + under) * (1.0 - kWil);
        col += wgt * S;
        dSum += wgt * t;
        hSum += wgt * p.y;
        wSum += wgt;
        T *= tr;
        t += dt;
    }
    var alpha = 1.0 - T;
    if (alpha < 0.002) {
        return vec4<f32>(0.0);
    }
    // Distance haze, from where the cloud was seen.
    let dist = dSum / max(wSum, 1e-6);
    let haze = 0.6 * (1.0 - exp(-dist / (1.5 * U.p[5].w)));
    // Soft shoulder instead of clipping at white: the sunny side stays modelled.
    var c = col / alpha;
    c = 1.0 - exp(-1.4 * c);
    c = mix(c, U.p[6].rgb * U.p[6].w, haze);
    // Haze is thickest near the ground: the low part of a far cloud goes into it like the land round it does.
    let hMean = hSum / max(wSum, 1e-6);
    let lowHaze = (1.0 - exp(-dist / 2600.0)) * exp(-max(hMean, 0.0) / 320.0);
    alpha *= 1.0 - 0.85 * lowHaze;
    // Past the end of the drawn terrain the base of a far cloud goes into the fog there, as the ground would hide it.
    var fade = 1.0;
    if (U.p[12].z >= -9.0) {
        fade = smoothstep(U.p[12].z - 0.003, U.p[12].z + 0.022, rd.y / max(length(rd.xz), 1e-6));
    }
    alpha *= fade;
    return vec4<f32>(c * alpha, alpha);
}

@fragment
fn fs_volume(v: VOut) -> @location(0) vec4<f32> {
    let ndc = vec2<f32>(v.uv.x * 2.0 - 1.0, 1.0 - v.uv.y * 2.0);
    let a = U.inv * vec4<f32>(ndc, 0.5, 1.0);
    let rd = normalize(a.xyz / a.w);
    // Stop at the terrain. The depth is reversed (1 near, 0 far and sky).
    let size = vec2<i32>(U.sizes.xy);
    let at = clamp(vec2<i32>(v.uv * U.sizes.xy), vec2<i32>(0), size - vec2<i32>(1));
    let depth = textureLoad(depth_tex, at, 0);
    var tMax = 1e9;
    if (depth > 0.0) {
        let q = U.inv * vec4<f32>(ndc, depth, 1.0);
        tMax = length(q.xyz / q.w);
    }
    let c = kilotonCloud(U.p[0].xyz, rd, tMax, v.pos.xy);
    if (c.a < 0.002) {
        discard;
    }
    return c;
}
