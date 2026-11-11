// GPU versions of the effect steps in effects.rs (see gpu.rs for how they are chained).
//
// Every frame-sized region lives in `buf`, one u32 per pixel (r | g << 8 | b << 16),
// except the I/O region, which holds packed RGB24 bytes. `p` says which regions a pass
// reads (src, aux) and writes (dst, aux2), and `dat` holds per-frame tables (the grain
// table at 0, then each pass's own data at `p.data`). Each pass runs one invocation per
// item, and does the same integer maths as the CPU code, so most effects match it
// bit for bit.

struct P {
    src: u32,
    dst: u32,
    aux: u32,
    aux2: u32,
    w: u32,
    h: u32,
    sw: u32,
    sh: u32,
    data: u32,
    count: u32,
    mode: u32,
    r: u32,
    i0: vec4<i32>,
    i1: vec4<i32>,
    i2: vec4<i32>,
    f0: vec4<f32>,
    f1: vec4<f32>,
}

@group(0) @binding(0) var<uniform> p: P;
@group(0) @binding(1) var<storage, read_write> buf: array<u32>;
@group(0) @binding(2) var<storage, read> dat: array<u32>;

var<private> BAYER: array<u32, 16> = array<u32, 16>(0u, 8u, 2u, 10u, 12u, 4u, 14u, 6u, 3u, 11u, 1u, 9u, 15u, 7u, 13u, 5u);

fn item(gid: vec3<u32>, groups: vec3<u32>) -> u32 {
    return gid.x + gid.y * groups.x * 256u;
}

fn rgb(v: u32) -> vec3<u32> {
    return vec3<u32>(v & 255u, (v >> 8u) & 255u, (v >> 16u) & 255u);
}

fn ld(base: u32, i: u32) -> vec3<u32> {
    return rgb(buf[base + i]);
}

// Stores keep the low 8 bits of each channel, like `as u8` on the CPU.
fn st(base: u32, i: u32, c: vec3<u32>) {
    let m = c & vec3<u32>(255u);
    buf[base + i] = m.x | (m.y << 8u) | (m.z << 16u);
}

fn sti(base: u32, i: u32, c: vec3<i32>) {
    st(base, i, vec3<u32>(c));
}

// The pixel at (x, y) of a p.w x p.h frame, with coordinates clamped to the edges.
fn at(base: u32, x: i32, y: i32) -> vec3<u32> {
    let cx = u32(clamp(x, 0, i32(p.w) - 1));
    let cy = u32(clamp(y, 0, i32(p.h) - 1));
    return ld(base, cy * p.w + cx);
}

fn luma(c: vec3<u32>) -> u32 {
    return (c.x * 77u + c.y * 150u + c.z * 29u) >> 8u;
}

fn grain(k: u32) -> i32 {
    return bitcast<i32>(dat[k]);
}

// Vignette factor (0..=256) at (x, y), as `vignette` in effects.rs.
fn vignette(x: u32, y: u32) -> u32 {
    let cx = f32(p.w) * 0.5;
    let cy = f32(p.h) * 0.5;
    let nx = (f32(x) - cx) / cx;
    let ny = (f32(y) - cy) / cy;
    return u32(max(1.0 - 0.55 * (nx * nx + ny * ny), 0.0) * 256.0);
}

// One byte of the packed RGB24 region at `base`.
fn byte_at(base: u32, b: u32) -> u32 {
    return (buf[base + (b >> 2u)] >> ((b & 3u) * 8u)) & 255u;
}

// RGB24 bytes (src) -> pixels (dst).
@compute @workgroup_size(256)
fn unpack(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) groups: vec3<u32>) {
    let i = item(gid, groups);
    if i >= p.count {
        return;
    }
    let b = i * 3u;
    st(p.dst, i, vec3<u32>(byte_at(p.src, b), byte_at(p.src, b + 1u), byte_at(p.src, b + 2u)));
}

// Pixels (src) -> RGB24 bytes (dst), one u32 word (4 bytes) per item. p.r is the
// number of bytes.
@compute @workgroup_size(256)
fn pack(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) groups: vec3<u32>) {
    let j = item(gid, groups);
    if j >= p.count {
        return;
    }
    var v = 0u;
    for (var k = 0u; k < 4u; k++) {
        let b = j * 4u + k;
        if b < p.r {
            let c = ld(p.src, b / 3u);
            v |= c[b % 3u] << (k * 8u);
        }
    }
    buf[p.dst + j] = v;
}

@compute @workgroup_size(256)
fn copy(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) groups: vec3<u32>) {
    let i = item(gid, groups);
    if i >= p.count {
        return;
    }
    buf[p.dst + i] = buf[p.src + i];
}

// `geometry`. mode bits: 1 mirror, 2 mirror_y, 4 per-pixel path, 8 identity_x,
// 16 bulge. f0 = (1 / zoom, dx, dy, bulge), f1 = (sin / zoom, cos / zoom, rx2, ry2).
@compute @workgroup_size(256)
fn warp(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) groups: vec3<u32>) {
    let i = item(gid, groups);
    if i >= p.count {
        return;
    }
    let x = i % p.w;
    let y = i / p.w;
    let fx = select(x, p.w - 1u - x, (p.mode & 1u) != 0u && x >= p.w / 2u);
    let fy = select(y, p.h - 1u - y, (p.mode & 2u) != 0u && y >= p.h / 2u);
    let cx = f32(p.w) * 0.5;
    let cy = f32(p.h) * 0.5;
    let wmax = f32(p.w - 1u);
    let hmax = f32(p.h - 1u);
    let inv = p.f0.x;
    let dx = p.f0.y;
    let dy = p.f0.z;
    var sx: u32;
    var sy: u32;
    if (p.mode & 4u) != 0u {
        let sn = p.f1.x;
        let cs = p.f1.y;
        let u = f32(fx) - cx;
        let v = f32(fy) - cy;
        let uc = u * cs;
        let us = u * sn;
        let av = -v * sn;
        let bv = v * cs;
        let ox = cx + dx;
        let oy = cy + dy;
        var su = uc + av + ox;
        var sv = us + bv + oy;
        if (p.mode & 16u) != 0u {
            let t = max(1.0 - (u * u * p.f1.z + v * v * p.f1.w), 0.0);
            let s = 1.0 - p.f0.w * t * t;
            su = s * (uc + av) + ox;
            sv = s * (us + bv) + oy;
        }
        sx = u32(clamp(su, 0.0, wmax));
        sy = u32(clamp(sv, 0.0, hmax));
    } else {
        sy = u32(clamp((f32(fy) - cy) * inv + cy + dy, 0.0, hmax));
        if (p.mode & 8u) != 0u {
            sx = x;
        } else {
            sx = u32(clamp((f32(fx) - cx) * inv + cx + dx, 0.0, wmax));
        }
    }
    buf[p.dst + i] = buf[p.src + sy * p.w + sx];
}

// `shift_row` for every row: dat[p.data + y] is the row's offset.
@compute @workgroup_size(256)
fn shift(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) groups: vec3<u32>) {
    let i = item(gid, groups);
    if i >= p.count {
        return;
    }
    let x = i32(i % p.w);
    let y = i / p.w;
    let off = bitcast<i32>(dat[p.data + y]);
    let sx = u32(clamp(x - off, 0, i32(p.w) - 1));
    buf[p.dst + i] = buf[p.src + y * p.w + sx];
}

// `blend` of src with aux by p.r / 256; with mode 1 the result also goes to aux2.
@compute @workgroup_size(256)
fn blend(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) groups: vec3<u32>) {
    let i = item(gid, groups);
    if i >= p.count {
        return;
    }
    let k = p.r;
    let o = (ld(p.src, i) * (256u - k) + ld(p.aux, i) * k) >> vec3<u32>(8u);
    st(p.dst, i, o);
    if p.mode == 1u {
        st(p.aux2, i, o);
    }
}

// `echo`: aux is lag1, aux2 is lag2.
@compute @workgroup_size(256)
fn echo(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) groups: vec3<u32>) {
    let i = item(gid, groups);
    if i >= p.count {
        return;
    }
    let k = p.r;
    let o = ld(p.src, i);
    let a = (ld(p.aux, i) * k + o * (256u - k)) >> vec3<u32>(8u);
    let b = (ld(p.aux2, i) * k + a * (256u - k)) >> vec3<u32>(8u);
    st(p.aux, i, a);
    st(p.aux2, i, b);
    st(p.dst, i, vec3<u32>(o.x, a.y, b.z));
}

// One pass of `box_blur` with radius p.r over a p.w x p.h frame; p.mode is the
// reciprocal of the window (x / div as a multiply and shift).
@compute @workgroup_size(256)
fn hblur(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) groups: vec3<u32>) {
    let i = item(gid, groups);
    if i >= p.count {
        return;
    }
    let x = i32(i % p.w);
    let y = i32(i / p.w);
    let r = i32(p.r);
    var s = vec3<u32>(0u);
    for (var k = -r; k <= r; k++) {
        s += at(p.src, x + k, y);
    }
    st(p.dst, i, (s * p.mode) >> vec3<u32>(16u));
}

@compute @workgroup_size(256)
fn vblur(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) groups: vec3<u32>) {
    let i = item(gid, groups);
    if i >= p.count {
        return;
    }
    let x = i32(i % p.w);
    let y = i32(i / p.w);
    let r = i32(p.r);
    var s = vec3<u32>(0u);
    for (var k = -r; k <= r; k++) {
        s += at(p.src, x, y + k);
    }
    st(p.dst, i, (s * p.mode) >> vec3<u32>(16u));
}

// `downsample` of the p.w x p.h src by p.r into the p.sw x p.sh dst. Mode 1 keeps
// only the bright part, as glow does.
@compute @workgroup_size(256)
fn down(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) groups: vec3<u32>) {
    let i = item(gid, groups);
    if i >= p.count {
        return;
    }
    let f = p.r;
    let x0 = (i % p.sw) * f;
    let y0 = (i / p.sw) * f;
    let x1 = min(x0 + f, p.w);
    let y1 = min(y0 + f, p.h);
    var s = vec3<u32>(0u);
    for (var y = y0; y < y1; y++) {
        for (var x = x0; x < x1; x++) {
            s += ld(p.src, y * p.w + x);
        }
    }
    var v = s / ((x1 - x0) * (y1 - y0));
    if p.mode == 1u {
        v = min((max(v, vec3<u32>(140u)) - 140u) * 2u, vec3<u32>(255u));
    }
    st(p.dst, i, v);
}

// Bilinear tap of output index i (of n) into m: the two source indices and the weight
// of the second (0..256), as `bilinear_taps`.
fn tap(i: u32, n: u32, m: u32) -> vec3<u32> {
    let f = clamp((f32(i) + 0.5) * f32(m) / f32(n) - 0.5, 0.0, f32(m - 1u));
    let a = u32(f);
    return vec3<u32>(a, min(a + 1u, m - 1u), u32((f - f32(a)) * 256.0));
}

// `upsample` of the p.sw x p.sh aux to p.w x p.h. Mode 0 replaces src, mode 1 adds
// the result times p.i0.x / 256 (glow).
@compute @workgroup_size(256)
fn up(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) groups: vec3<u32>) {
    let i = item(gid, groups);
    if i >= p.count {
        return;
    }
    let tx = tap(i % p.w, p.w, p.sw);
    let ty = tap(i / p.w, p.h, p.sh);
    let wx = tx.z;
    let wy = ty.z;
    let top = ld(p.aux, ty.x * p.sw + tx.x) * (256u - wx) + ld(p.aux, ty.x * p.sw + tx.y) * wx;
    let bot = ld(p.aux, ty.y * p.sw + tx.x) * (256u - wx) + ld(p.aux, ty.y * p.sw + tx.y) * wx;
    var v = (top * (256u - wy) + bot * wy) >> vec3<u32>(16u);
    if p.mode == 1u {
        v = min(ld(p.src, i) + ((v * u32(p.i0.x)) >> vec3<u32>(8u)), vec3<u32>(255u));
    }
    st(p.dst, i, v);
}

// `rgb_split` by p.i0.x.
@compute @workgroup_size(256)
fn split(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) groups: vec3<u32>) {
    let i = item(gid, groups);
    if i >= p.count {
        return;
    }
    let x = i32(i % p.w);
    let y = i32(i / p.w);
    let off = p.i0.x;
    let voff = off / 3;
    let c = ld(p.src, i);
    let r = at(p.src, x + off, y + voff).x;
    let b = at(p.src, x - off, y - voff).z;
    st(p.dst, i, vec3<u32>(r, c.y, b));
}

// `grade`: mix with the table at p.data by p.r / 256; mode 1 adds the vignette.
@compute @workgroup_size(256)
fn grade(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) groups: vec3<u32>) {
    let i = item(gid, groups);
    if i >= p.count {
        return;
    }
    let c = ld(p.src, i);
    let t = rgb(dat[p.data + luma(c)]);
    var g = (c * (256u - p.r) + t * p.r) >> vec3<u32>(8u);
    if p.mode == 1u {
        g = (g * vignette(i % p.w, i / p.w)) >> vec3<u32>(8u);
    }
    st(p.dst, i, g);
}

// `hue_apply`: the matrix rows are i0.xyz, i1.xyz, i2.xyz.
@compute @workgroup_size(256)
fn hue(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) groups: vec3<u32>) {
    let i = item(gid, groups);
    if i >= p.count {
        return;
    }
    let c = vec3<i32>(ld(p.src, i));
    let o = vec3<i32>(dot(c, p.i0.xyz), dot(c, p.i1.xyz), dot(c, p.i2.xyz)) >> vec3<u32>(8u);
    sti(p.dst, i, clamp(o, vec3<i32>(0), vec3<i32>(255)));
}

// `swap` in channel order p.r.
@compute @workgroup_size(256)
fn swap(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) groups: vec3<u32>) {
    let i = item(gid, groups);
    if i >= p.count {
        return;
    }
    let c = ld(p.src, i);
    var o = c;
    switch p.r {
        case 1u: { o = c.yzx; }
        case 2u: { o = c.zxy; }
        case 3u: { o = c.xzy; }
        case 4u: { o = c.zyx; }
        case 5u: { o = c.yxz; }
        default: {}
    }
    st(p.dst, i, o);
}

// `glitch`. At p.data: slice, block and row counts, then 4 words per slice (y0, y1,
// shift, perm), 6 per block (sx, sy, dx, dy, w, h), and 1 + p.w per row (y, grays).
@compute @workgroup_size(256)
fn glitch(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) groups: vec3<u32>) {
    let i = item(gid, groups);
    if i >= p.count {
        return;
    }
    let x = i % p.w;
    let y = i / p.w;
    let w = i32(p.w);
    var c = ld(p.src, i);
    var k = p.data + 3u;
    for (var s = 0u; s < dat[p.data]; s++) {
        if y >= dat[k] && y < dat[k + 1u] {
            let shift = bitcast<i32>(dat[k + 2u]);
            let sx = u32(((i32(x) - shift) % w + w) % w);
            let t = ld(p.src, y * p.w + sx);
            switch dat[k + 3u] {
                case 0u: { c = t; }
                case 1u: { c = t.yzx; }
                default: { c = t.zxy; }
            }
        }
        k += 4u;
    }
    for (var b = 0u; b < dat[p.data + 1u]; b++) {
        let sx = dat[k];
        let sy = dat[k + 1u];
        let dx = dat[k + 2u];
        let dy = dat[k + 3u];
        if x >= dx && x < dx + dat[k + 4u] && y >= dy && y < dy + dat[k + 5u] {
            let x0 = sx + x - dx;
            let y0 = sy + y - dy;
            if x0 < p.w && y0 < p.h {
                c = ld(p.src, y0 * p.w + x0);
            }
        }
        k += 6u;
    }
    for (var r = 0u; r < dat[p.data + 2u]; r++) {
        if dat[k] == y {
            c = vec3<u32>(dat[k + 1u + x]);
        }
        k += 1u + p.w;
    }
    st(p.dst, i, c);
}

// `snow`: mixed in by p.i0.x / 256; dat[p.data + y] is each row's start in the grain.
@compute @workgroup_size(256)
fn snow(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) groups: vec3<u32>) {
    let i = item(gid, groups);
    if i >= p.count {
        return;
    }
    let x = i % p.w;
    let y = i / p.w;
    let a = p.i0.x;
    let n = grain(dat[p.data + y] + x);
    let s = ((n + 256) >> 1u) * a;
    sti(p.dst, i, (vec3<i32>(ld(p.src, i)) * (256 - a) + s) >> vec3<u32>(8u));
}

// `edges` with gain p.i0.x.
@compute @workgroup_size(256)
fn edges(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) groups: vec3<u32>) {
    let i = item(gid, groups);
    if i >= p.count {
        return;
    }
    let x = i32(i % p.w);
    let y = i32(i / p.w);
    let ua = i32(luma(at(p.src, x - 1, y - 1)));
    let ub = i32(luma(at(p.src, x, y - 1)));
    let uc = i32(luma(at(p.src, x + 1, y - 1)));
    let ma = i32(luma(at(p.src, x - 1, y)));
    let mc = i32(luma(at(p.src, x + 1, y)));
    let da = i32(luma(at(p.src, x - 1, y + 1)));
    let db = i32(luma(at(p.src, x, y + 1)));
    let dc = i32(luma(at(p.src, x + 1, y + 1)));
    let gx = uc - ua + 2 * (mc - ma) + dc - da;
    let gy = da + 2 * db + dc - ua - 2 * ub - uc;
    let ax = abs(gx);
    let ay = abs(gy);
    let e = u32(min(((max(ax, ay) + (min(ax, ay) >> 1u)) * p.i0.x) >> 10u, 256));
    let c = ld(p.src, i);
    st(p.dst, i, min((((c >> vec3<u32>(1u)) + 64u) * e) >> vec3<u32>(7u), vec3<u32>(255u)));
}

// `pixelate` with blocks of p.r.
@compute @workgroup_size(256)
fn pixel(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) groups: vec3<u32>) {
    let i = item(gid, groups);
    if i >= p.count {
        return;
    }
    let b = p.r;
    let cx = min((i % p.w) / b * b + b / 2u, p.w - 1u);
    let cy = min((i / p.w) / b * b + b / 2u, p.h - 1u);
    buf[p.dst + i] = buf[p.src + cy * p.w + cx];
}

// `dither` to p.r levels, with Bayer cells of p.i0.x pixels.
@compute @workgroup_size(256)
fn dither(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) groups: vec3<u32>) {
    let i = item(gid, groups);
    if i >= p.count {
        return;
    }
    let cell = u32(p.i0.x);
    let b = BAYER[((i / p.w / cell) % 4u) * 4u + (i % p.w / cell) % 4u];
    let t = vec3<u32>(b * 16u + 8u);
    let steps = max(p.r, 2u) - 1u;
    let s = ld(p.src, i) * steps;
    let q = s / 255u + select(vec3<u32>(0u), vec3<u32>(1u), s % 255u > t);
    st(p.dst, i, q * 255u / steps);
}

// `vhs`: scanlines every p.r rows, grain p.i0.x; at p.data, 2 words per row (shift,
// start in the grain).
@compute @workgroup_size(256)
fn vhs(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) groups: vec3<u32>) {
    let i = item(gid, groups);
    if i >= p.count {
        return;
    }
    let x = i % p.w;
    let y = i / p.w;
    let k = p.data + y * 2u;
    let c = vec3<i32>(at(p.src, i32(x) - bitcast<i32>(dat[k]), i32(y)));
    let scan = select(256, 210, (y / p.r) % 2u == 1u);
    let n = grain(dat[k + 1u] + x) * p.i0.x / 256;
    sti(p.dst, i, clamp(((c * scan) >> vec3<u32>(8u)) + n, vec3<i32>(0), vec3<i32>(255)));
}

// `crt`: scanlines every p.r rows, phosphor cells of p.i0.x, boost p.i0.y.
@compute @workgroup_size(256)
fn crt(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) groups: vec3<u32>) {
    let i = item(gid, groups);
    if i >= p.count {
        return;
    }
    let x = i % p.w;
    let y = i / p.w;
    let scan = select(256u, 150u, (y / p.r) % 2u == 1u);
    let k = (scan * u32(p.i0.y)) >> 8u;
    let lit = (x / u32(p.i0.x)) % 3u;
    let m = vec3<u32>(select(170u, 256u, lit == 0u), select(170u, 256u, lit == 1u), select(170u, 256u, lit == 2u));
    let gain = (m * k) >> vec3<u32>(8u);
    st(p.dst, i, min((ld(p.src, i) * gain) >> vec3<u32>(8u), vec3<u32>(255u)));
}

// `film`: sepia table at p.data; i0 = (mix, grain, rows, scratches), i1 = (dust, size).
// Scratches: a count, then columns. Dust: a count, then (x, y) pairs.
@compute @workgroup_size(256)
fn film(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) groups: vec3<u32>) {
    let i = item(gid, groups);
    if i >= p.count {
        return;
    }
    let x = i % p.w;
    let y = i / p.w;
    let mix = p.i0.x;
    let c = ld(p.src, i);
    let t = vec3<i32>(rgb(dat[p.data + luma(c)])) * mix;
    let n = (grain(dat[u32(p.i0.z) + y] + x) * p.i0.y) >> 8u;
    var o = clamp(((vec3<i32>(c) * (256 - mix) + t) >> vec3<u32>(8u)) + n, vec3<i32>(0), vec3<i32>(255));
    let sb = u32(p.i0.w);
    for (var s = 0u; s < dat[sb]; s++) {
        if dat[sb + 1u + s] == x {
            o = vec3<i32>(230, 220, 200);
        }
    }
    let db = u32(p.i1.x);
    let size = u32(p.i1.y);
    for (var d = 0u; d < dat[db]; d++) {
        let x0 = dat[db + 1u + d * 2u];
        let y0 = dat[db + 2u + d * 2u];
        if x >= x0 && x < x0 + size && y >= y0 && y < y0 + size {
            o = vec3<i32>(15, 10, 5);
        }
    }
    sti(p.dst, i, o);
}

// `darken_edges` by p.r / 256.
@compute @workgroup_size(256)
fn darken(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) groups: vec3<u32>) {
    let i = item(gid, groups);
    if i >= p.count {
        return;
    }
    let dark = min(((256u - vignette(i % p.w, i / p.w)) * p.r) >> 8u, 256u);
    st(p.dst, i, (ld(p.src, i) * (256u - dark)) >> vec3<u32>(8u));
}

// Cinema bars of p.r rows.
@compute @workgroup_size(256)
fn cinema(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) groups: vec3<u32>) {
    let i = item(gid, groups);
    if i >= p.count {
        return;
    }
    let y = i / p.w;
    buf[p.dst + i] = select(buf[p.src + i], 0u, y < p.r || y >= p.h - p.r);
}

// One curve (256 words at p.data) for every channel.
@compute @workgroup_size(256)
fn tone(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) groups: vec3<u32>) {
    let i = item(gid, groups);
    if i >= p.count {
        return;
    }
    let c = ld(p.src, i);
    st(p.dst, i, vec3<u32>(dat[p.data + c.x], dat[p.data + c.y], dat[p.data + c.z]));
}
