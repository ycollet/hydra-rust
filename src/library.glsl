float _luminance(vec3 rgb) {
  const vec3 W = vec3(0.2125, 0.7154, 0.0721);
  return dot(rgb, W);
}

// Simplex 3D Noise — Ian McEwan, Ashima Arts
vec4 permute(vec4 x) {
  return mod(((x * 34.0) + 1.0) * x, 289.0);
}
vec4 taylorInvSqrt(vec4 r) {
  return 1.79284291400159 - 0.85373472095314 * r;
}

float _noise(vec3 v) {
  const vec2 C = vec2(1.0 / 6.0, 1.0 / 3.0);
  const vec4 D = vec4(0.0, 0.5, 1.0, 2.0);
  vec3 i = floor(v + dot(v, C.yyy));
  vec3 x0 = v - i + dot(i, C.xxx);
  vec3 g = step(x0.yzx, x0.xyz);
  vec3 l = 1.0 - g;
  vec3 i1 = min(g.xyz, l.zxy);
  vec3 i2 = max(g.xyz, l.zxy);
  vec3 x1 = x0 - i1 + 1.0 * C.xxx;
  vec3 x2 = x0 - i2 + 2.0 * C.xxx;
  vec3 x3 = x0 - 1.0 + 3.0 * C.xxx;
  i = mod(i, 289.0);
  vec4 p = permute(permute(permute(
    i.z + vec4(0.0, i1.z, i2.z, 1.0))
    + i.y + vec4(0.0, i1.y, i2.y, 1.0))
    + i.x + vec4(0.0, i1.x, i2.x, 1.0));
  float n_ = 1.0 / 7.0;
  vec3 ns = n_ * D.wyz - D.xzx;
  vec4 j = p - 49.0 * floor(p * ns.z * ns.z);
  vec4 x_ = floor(j * ns.z);
  vec4 y_ = floor(j - 7.0 * x_);
  vec4 x = x_ * ns.x + ns.yyyy;
  vec4 y = y_ * ns.x + ns.yyyy;
  vec4 h = 1.0 - abs(x) - abs(y);
  vec4 b0 = vec4(x.xy, y.xy);
  vec4 b1 = vec4(x.zw, y.zw);
  vec4 s0 = floor(b0) * 2.0 + 1.0;
  vec4 s1 = floor(b1) * 2.0 + 1.0;
  vec4 sh = -step(h, vec4(0.0));
  vec4 a0 = b0.xzyw + s0.xzyw * sh.xxyy;
  vec4 a1 = b1.xzyw + s1.xzyw * sh.zzww;
  vec3 p0 = vec3(a0.xy, h.x);
  vec3 p1 = vec3(a0.zw, h.y);
  vec3 p2 = vec3(a1.xy, h.z);
  vec3 p3 = vec3(a1.zw, h.w);
  vec4 norm = taylorInvSqrt(vec4(dot(p0,p0), dot(p1,p1), dot(p2,p2), dot(p3,p3)));
  p0 *= norm.x; p1 *= norm.y; p2 *= norm.z; p3 *= norm.w;
  vec4 m = max(0.6 - vec4(dot(x0,x0), dot(x1,x1), dot(x2,x2), dot(x3,x3)), 0.0);
  m = m * m;
  return 42.0 * dot(m*m, vec4(dot(p0,x0), dot(p1,x1), dot(p2,x2), dot(p3,x3)));
}

vec3 _rgbToHsv(vec3 c) {
  vec4 K = vec4(0.0, -1.0/3.0, 2.0/3.0, -1.0);
  vec4 p = mix(vec4(c.bg, K.wz), vec4(c.gb, K.xy), step(c.b, c.g));
  vec4 q = mix(vec4(p.xyw, c.r), vec4(c.r, p.yzx), step(p.x, c.r));
  float d = q.x - min(q.w, q.y);
  float e = 1.0e-10;
  return vec3(abs(q.z + (q.w - q.y) / (6.0 * d + e)), d / (q.x + e), q.x);
}

vec3 _hsvToRgb(vec3 c) {
  vec4 K = vec4(1.0, 2.0/3.0, 1.0/3.0, 3.0);
  vec3 p = abs(fract(c.xxx + K.xyz) * 6.0 - K.www);
  return c.z * mix(K.xxx, clamp(p - K.xxx, 0.0, 1.0), c.y);
}

vec4 osc(vec2 _st, float frequency, float sync, float offset) {
  float r = sin((_st.x - offset/frequency + iTime*sync) * frequency) * 0.5 + 0.5;
  float g = sin((_st.x + iTime*sync) * frequency) * 0.5 + 0.5;
  float b = sin((_st.x + offset/frequency + iTime*sync) * frequency) * 0.5 + 0.5;
  return vec4(r, g, b, 1.0);
}

vec4 noise(vec2 _st, float scale, float offset) {
  return vec4(vec3(_noise(vec3(_st * scale, offset * iTime))), 1.0);
}

vec4 voronoi(vec2 _st, float scale, float speed, float blending) {
  vec3 color = vec3(0.0);
  _st *= scale;
  vec2 i_st = floor(_st);
  vec2 f_st = fract(_st);
  float m_dist = 10.0;
  vec2 m_point;
  for (int j = -1; j <= 1; j++) {
    for (int i = -1; i <= 1; i++) {
      vec2 neighbor = vec2(float(i), float(j));
      vec2 p = i_st + neighbor;
      vec2 point = fract(sin(vec2(dot(p, vec2(127.1,311.7)), dot(p, vec2(269.5,183.3)))) * 43758.5453);
      point = 0.5 + 0.5 * sin(iTime * speed + 6.2831 * point);
      vec2 diff = neighbor + point - f_st;
      float dist = length(diff);
      if (dist < m_dist) {
        m_dist = dist;
        m_point = point;
      }
    }
  }
  color += dot(m_point, vec2(0.3, 0.6));
  color *= 1.0 - blending * m_dist;
  return vec4(color, 1.0);
}

vec4 shape(vec2 _st, float sides, float radius, float smoothing) {
  vec2 st = _st * 2.0 - 1.0;
  float a = atan(st.x, st.y) + 3.1416;
  float r = (2.0 * 3.1416) / sides;
  float d = cos(floor(0.5 + a/r) * r - a) * length(st);
  return vec4(vec3(1.0 - smoothstep(radius, radius + smoothing + 0.0000001, d)), 1.0);
}

vec4 gradient(vec2 _st, float speed) {
  return vec4(_st, sin(iTime * speed), 1.0);
}

vec4 solid(vec2 _st, float r, float g, float b, float a) {
  return vec4(r, g, b, a);
}

vec4 rings(vec2 _st, float freq, float speed) {
  float d = length(_st - 0.5);
  float v = sin((d * freq - iTime * speed) * 6.2832) * 0.5 + 0.5;
  return vec4(vec3(v), 1.0);
}

vec4 checker(vec2 _st, float cols, float rows) {
  float v = mod(floor(_st.x * cols) + floor(_st.y * rows), 2.0);
  return vec4(vec3(v), 1.0);
}

vec2 rotate(vec2 _st, float angle, float speed) {
  vec2 xy = _st - vec2(0.5);
  float ang = angle + speed * iTime;
  xy = mat2(cos(ang), -sin(ang), sin(ang), cos(ang)) * xy;
  xy += 0.5;
  return xy;
}

vec2 scale(vec2 _st, float amount, float xMult, float yMult, float offsetX, float offsetY) {
  vec2 xy = _st - vec2(offsetX, offsetY);
  xy *= (1.0 / vec2(amount * xMult, amount * yMult));
  xy += vec2(offsetX, offsetY);
  return xy;
}

vec4 color(vec4 _c0, float r, float g, float b, float a) {
  vec4 c = vec4(r, g, b, a);
  vec4 pos = step(0.0, c);
  return vec4(mix((1.0 - _c0) * abs(c), c * _c0, pos));
}

vec4 invert(vec4 _c0, float amount) {
  return vec4((1.0 - _c0.rgb) * amount + _c0.rgb * (1.0 - amount), _c0.a);
}

vec4 contrast(vec4 _c0, float amount) {
  vec4 c = (_c0 - vec4(0.5)) * vec4(amount) + vec4(0.5);
  return vec4(c.rgb, _c0.a);
}

vec4 brightness(vec4 _c0, float amount) {
  return vec4(_c0.rgb + vec3(amount), _c0.a);
}

vec4 saturate(vec4 _c0, float amount) {
  const vec3 W = vec3(0.2125, 0.7154, 0.0721);
  vec3 intensity = vec3(dot(_c0.rgb, W));
  return vec4(mix(intensity, _c0.rgb, amount), _c0.a);
}

vec4 hue(vec4 _c0, float hue) {
  vec3 c = _rgbToHsv(_c0.rgb);
  c.r += hue;
  return vec4(_hsvToRgb(c), _c0.a);
}

vec4 posterize(vec4 _c0, float bins, float gamma) {
  vec4 c2 = pow(_c0, vec4(gamma));
  c2 *= vec4(bins);
  c2 = floor(c2);
  c2 /= vec4(bins);
  c2 = pow(c2, vec4(1.0/gamma));
  return vec4(c2.xyz, _c0.a);
}

vec4 luma(vec4 _c0, float threshold, float tolerance) {
  float a = smoothstep(threshold-(tolerance+0.0000001), threshold+(tolerance+0.0000001), _luminance(_c0.rgb));
  return vec4(_c0.rgb * a, a);
}

vec4 colorama(vec4 _c0, float amount) {
  vec3 c = _rgbToHsv(_c0.rgb);
  c += vec3(amount);
  c = _hsvToRgb(c);
  c = fract(c);
  return vec4(c, _c0.a);
}

vec2 scroll(vec2 _st, float scrollX, float scrollY, float speedX, float speedY) {
  _st.x += scrollX + iTime * speedX;
  _st.y += scrollY + iTime * speedY;
  return fract(_st);
}

vec2 kaleid(vec2 _st, float nSides) {
  vec2 st = _st - 0.5;
  float r = length(st);
  float a = atan(st.y, st.x);
  float pi = 2.0 * 3.1416;
  a = mod(a, pi / nSides);
  a = abs(a - pi / nSides / 2.0);
  return r * vec2(cos(a), sin(a));
}

vec2 pixelate(vec2 _st, float pixelX, float pixelY) {
  vec2 xy = vec2(pixelX, pixelY);
  return (floor(_st * xy) + 0.5) / xy;
}

vec2 repeat(vec2 _st, float repeatX, float repeatY, float offsetX, float offsetY) {
  vec2 st = _st * vec2(repeatX, repeatY);
  st.x += step(1.0, mod(st.y, 2.0)) * offsetX;
  st.y += step(1.0, mod(st.x, 2.0)) * offsetY;
  return fract(st);
}

vec2 modulate(vec2 _st, vec4 _c0, float amount) {
  return _st + _c0.xy * amount;
}

vec2 modulateScale(vec2 _st, vec4 _c0, float multiple, float offset) {
  vec2 xy = _st - vec2(0.5);
  xy *= (1.0 / vec2(offset + multiple * _c0.r, offset + multiple * _c0.g));
  xy += vec2(0.5);
  return xy;
}

vec2 modulateRotate(vec2 _st, vec4 _c0, float multiple, float offset) {
  vec2 xy = _st - vec2(0.5);
  float angle = offset + _c0.x * multiple;
  xy = mat2(cos(angle), -sin(angle), sin(angle), cos(angle)) * xy;
  xy += 0.5;
  return xy;
}

vec4 add(vec4 _c0, vec4 _c1, float amount) {
  return (_c0 + _c1) * amount + _c0 * (1.0 - amount);
}

vec4 mult(vec4 _c0, vec4 _c1, float amount) {
  return _c0 * (1.0 - amount) + (_c0 * _c1) * amount;
}

vec4 blend(vec4 _c0, vec4 _c1, float amount) {
  return _c0 * (1.0 - amount) + _c1 * amount;
}

vec4 diff(vec4 _c0, vec4 _c1) {
  return vec4(abs(_c0.rgb - _c1.rgb), max(_c0.a, _c1.a));
}

vec4 layer(vec4 _c0, vec4 _c1) {
  return vec4(mix(_c0.rgb, _c1.rgb, _c1.a), clamp(_c0.a + _c1.a, 0.0, 1.0));
}

vec4 mask(vec4 _c0, vec4 _c1) {
  float a = _luminance(_c1.rgb);
  return vec4(_c0.rgb * a, a * _c0.a);
}

vec2 scrollX(vec2 _st, float scrollX, float speed) {
  _st.x += scrollX + iTime * speed;
  return fract(_st);
}

vec2 scrollY(vec2 _st, float scrollY, float speed) {
  _st.y += scrollY + iTime * speed;
  return fract(_st);
}

vec2 repeatX(vec2 _st, float reps, float offset) {
  vec2 st = _st * vec2(reps, 1.0);
  st.y += step(1.0, mod(st.x, 2.0)) * offset;
  return fract(st);
}

vec2 repeatY(vec2 _st, float reps, float offset) {
  vec2 st = _st * vec2(1.0, reps);
  st.x += step(1.0, mod(st.y, 2.0)) * offset;
  return fract(st);
}

vec2 polar(vec2 _st) {
  vec2 p = _st - 0.5;
  float r = length(p) * 2.0;
  float a = atan(p.y, p.x) / 6.2832 + 0.5;
  return vec2(a, r);
}

vec2 cart(vec2 _st) {
  float a = (_st.x - 0.5) * 6.2832;
  float r = _st.y * 0.5;
  return vec2(cos(a), sin(a)) * r + 0.5;
}

vec2 fold(vec2 _st, float amount) {
  vec2 p = (_st - 0.5) * (1.0 + amount);
  p = abs(p);
  p = fract(p);
  return p;
}

vec4 shift(vec4 _c0, float r, float g, float b, float a) {
  return vec4(fract(_c0.r + r), fract(_c0.g + g), fract(_c0.b + b), clamp(_c0.a + a, 0.0, 1.0));
}

vec4 thresh(vec4 _c0, float threshold, float tolerance) {
  float a = smoothstep(threshold - tolerance, threshold + tolerance, _luminance(_c0.rgb));
  return vec4(vec3(a), _c0.a);
}

vec4 r(vec4 _c0, float scale, float offset) {
  return vec4(_c0.r * scale + offset, _c0.g, _c0.b, _c0.a);
}

vec4 g(vec4 _c0, float scale, float offset) {
  return vec4(_c0.r, _c0.g * scale + offset, _c0.b, _c0.a);
}

vec4 b(vec4 _c0, float scale, float offset) {
  return vec4(_c0.r, _c0.g, _c0.b * scale + offset, _c0.a);
}

vec4 a(vec4 _c0, float scale, float offset) {
  return vec4(_c0.a * scale + offset);
}

vec4 sum(vec4 _c0, float scaleR, float scaleG, float scaleB, float scaleA) {
  vec4 v = _c0 * vec4(scaleR, scaleG, scaleB, scaleA);
  float s = v.r + v.g + v.b + v.a;
  return vec4(vec3(s), _c0.a);
}

vec4 sub(vec4 _c0, vec4 _c1, float amount) {
  return (_c0 - _c1) * amount + _c0 * (1.0 - amount);
}

vec2 modulateRepeat(vec2 _st, vec4 _c0, float repeatX, float repeatY, float offsetX, float offsetY) {
  vec2 st = _st * vec2(repeatX, repeatY);
  st.x += step(1.0, mod(st.y, 2.0)) + _c0.r * offsetX;
  st.y += step(1.0, mod(st.x, 2.0)) + _c0.g * offsetY;
  return fract(st);
}

vec2 modulateRepeatX(vec2 _st, vec4 _c0, float reps, float offset) {
  vec2 st = _st * vec2(reps, 1.0);
  st.y += step(1.0, mod(st.x, 2.0)) + _c0.r * offset;
  return fract(st);
}

vec2 modulateRepeatY(vec2 _st, vec4 _c0, float reps, float offset) {
  vec2 st = _st * vec2(1.0, reps);
  st.x += step(1.0, mod(st.y, 2.0)) + _c0.r * offset;
  return fract(st);
}

vec2 modulateKaleid(vec2 _st, vec4 _c0, float nSides) {
  vec2 st = _st - 0.5;
  float r = length(st);
  float a = atan(st.y, st.x);
  float pi = 2.0 * 3.1416;
  a = mod(a, pi / nSides);
  a = abs(a - pi / nSides / 2.0);
  return (_c0.r + r) * vec2(cos(a), sin(a));
}

vec2 modulateScrollX(vec2 _st, vec4 _c0, float scrollX, float speed) {
  _st.x += _c0.r * scrollX + iTime * speed;
  return fract(_st);
}

vec2 modulateScrollY(vec2 _st, vec4 _c0, float scrollY, float speed) {
  _st.y += _c0.r * scrollY + iTime * speed;
  return fract(_st);
}

vec2 modulatePixelate(vec2 _st, vec4 _c0, float multiple, float offset) {
  vec2 xy = vec2(_c0.r * multiple + offset, _c0.g * multiple + offset);
  return (floor(_st * xy) + 0.5) / xy;
}

vec2 modulateHue(vec2 _st, vec4 _c0, float amount) {
  return _st + (vec2(_c0.g - _c0.r, _c0.b - _c0.g) * amount * (1.0 / iResolution));
}

// Ported from popular community extensions loaded via loadScript() in real
// hydra.js sketches (see SPEC.md §4, jsfunctions/loadScript). loadScript()
// itself stays a no-op (no dynamic module loading), but these specific
// functions are common enough to be worth porting natively.

// spiral, from metagrowing/extra-shaders-for-hydra (lib/lib-pattern.js),
// AGPL-3.0, by Thomas Jourdan.
vec4 spiral(vec2 _st, float a, float b, float thickness) {
  vec2 center = _st - vec2(0.5);
  float thick = clamp(thickness, 0.0, 1.0);
  float phi = atan(center.y, center.x) / 6.283185307179586 + 0.5;
  float r = length(center);
  float w = mod(a * phi - b * r, 1.0);
  const float epsilon = 0.00001;
  float d = smoothstep(epsilon, epsilon, w)
          - smoothstep(thick - epsilon, thick + epsilon, w);
  return vec4(d, d, d, 1.0);
}

// turb, from metagrowing/extra-shaders-for-hydra (lib/lib-noise.js),
// AGPL-3.0, by Thomas Jourdan.
vec4 turb(vec2 _st, float scale, float offset, float octaves) {
  int on = int(abs(octaves));
  float fr = fract(octaves);
  vec2 pos = scale * _st;
  float sc = 1.0;
  float fbm = 0.0;
  for (int io = 0; io < 8; io++) {
    fbm += sc * _noise(vec3(pos, offset * iTime));
    pos *= 2.0;
    sc /= 2.0;
    if (io >= on) break;
  }
  fbm += fr * sc * _noise(vec3(pos, offset * iTime));
  return vec4(fbm, fbm, fbm, 1.0);
}

// inversion, from geikha/hyper-hydra (hydra-fractals.js), MIT license.
vec2 inversion(vec2 _st) {
  _st /= dot(_st, _st);
  return _st;
}

// colreflect, from metagrowing/extra-shaders-for-hydra (lib/lib-color.js),
// AGPL-3.0, by Thomas Jourdan.
vec4 colreflect(vec4 _c0, vec4 _c1, float amount) {
  vec3 cc = cross(_c0.rgb, normalize(_c1.rgb));
  return vec4(amount * cc + (1.0 - amount) * _c0.rgb, _c0.a);
}

// uturb, from metagrowing/extra-shaders-for-hydra (lib/lib-noise.js),
// AGPL-3.0, by Thomas Jourdan.
vec4 uturb(vec2 _st, float scale, float offset, float octaves) {
  int on = int(abs(octaves));
  float fr = fract(octaves);
  vec2 pos = scale * _st;
  float sc = 1.0;
  float fbm = 0.0;
  for (int io = 0; io < 8; io++) {
    fbm += sc * _noise(vec3(pos, offset * iTime));
    pos *= 2.0;
    sc /= 2.0;
    if (io >= on) break;
  }
  fbm += fr * sc * _noise(vec3(pos, offset * iTime));
  fbm = 0.5 + 0.5 * fbm;
  return vec4(fbm, fbm, fbm, 1.0);
}

// unoise, from metagrowing/extra-shaders-for-hydra (lib/lib-noise.js),
// AGPL-3.0, by Thomas Jourdan.
vec4 unoise(vec2 _st, float scale, float offset) {
  float noi = _noise(vec3(_st * scale, offset * iTime));
  noi = 0.5 + 0.5 * noi;
  return vec4(noi, noi, noi, 1.0);
}

// whitenoise, from metagrowing/extra-shaders-for-hydra (lib/lib-noise.js),
// AGPL-3.0, by Thomas Jourdan.
vec4 whitenoise(vec2 _st, float size, float dynamic) {
  const highp float wa = 12.9898;
  const highp float wb = 78.233;
  const highp float wc = 43758.5453;
  highp float dt = dot(floor((_st * iResolution) / size), vec2(dynamic * iTime) + vec2(wa, wb));
  highp float sn = mod(dt, 3.141592653589793);
  highp float d = fract(sin(sn) * wc);
  return vec4(d, d, d, 1.0);
}

// colornoise, from metagrowing/extra-shaders-for-hydra (lib/lib-noise.js),
// AGPL-3.0, by Thomas Jourdan.
vec4 colornoise(vec2 _st, float size, float dynamic) {
  highp float rr;
  highp float gg;
  highp float bb;
  {
    const highp float wa = 12.9898;
    const highp float wb = 78.233;
    const highp float wc = 43758.5453;
    highp float dt = dot(floor((_st * iResolution) / size), vec2(dynamic * iTime) + vec2(wa, wb));
    highp float sn = mod(dt, 3.141592653589793);
    rr = fract(sin(sn) * wc);
  }
  {
    const highp float wa = 12.9898;
    const highp float wb = 78.233;
    const highp float wc = 43758.5453;
    highp float dt = dot(floor((_st * iResolution) / size) + vec2(0.123, 0.567), vec2(dynamic * iTime) + vec2(wa, wb));
    highp float sn = mod(dt, 3.141592653589793);
    gg = fract(sin(sn) * wc);
  }
  {
    const highp float wa = 12.9898;
    const highp float wb = 78.233;
    const highp float wc = 43758.5453;
    highp float dt = dot(floor((_st * iResolution) / size) + vec2(0.543, 0.905), vec2(dynamic * iTime) + vec2(wa, wb));
    highp float sn = mod(dt, 3.141592653589793);
    bb = fract(sin(sn) * wc);
  }
  return vec4(rr, gg, bb, 1.0);
}

// warp, from metagrowing/extra-shaders-for-hydra (lib/lib-noise.js),
// AGPL-3.0, by Thomas Jourdan (inspired by Inigo Quilez's domain warping).
vec4 warp(vec2 _st, float scalei, float offset, float octaves, float octavesinner, float scale) {
  int oin = int(abs(octavesinner));
  float fri = fract(octavesinner);
  float fbmx = 0.0;
  {
    vec2 pos = scalei * _st;
    float sc = 1.0;
    for (int io = 0; io < 8; io++) {
      fbmx += sc * _noise(vec3(pos, offset * iTime));
      pos *= 2.0;
      sc /= 2.0;
      if (io >= oin) break;
    }
    fbmx += fri * sc * _noise(vec3(pos, offset * iTime));
  }
  float fbmy = 0.0;
  {
    vec2 pos = scalei * (_st + vec2(5.123, 3.987));
    float sc = 1.0;
    for (int io = 0; io < 8; io++) {
      fbmy += sc * _noise(vec3(pos, offset * iTime));
      pos *= 2.0;
      sc /= 2.0;
      if (io >= oin) break;
    }
    fbmy += fri * sc * _noise(vec3(pos, offset * iTime));
  }
  int on = int(abs(octaves));
  float fr = fract(octaves);
  float fbm = 0.0;
  vec2 pos = scale * vec2(fbmx, fbmy);
  float sc = 1.0;
  for (int io = 0; io < 8; io++) {
    fbm += sc * _noise(vec3(pos, offset * iTime));
    pos *= 2.0;
    sc /= 2.0;
    if (io >= on) break;
  }
  fbm += fr * sc * _noise(vec3(pos, offset * iTime));
  return vec4(fbm, fbm, fbm, 1.0);
}

// cwarp, from metagrowing/extra-shaders-for-hydra (lib/lib-noise.js),
// AGPL-3.0, by Thomas Jourdan.
vec4 cwarp(vec2 _st, float scalei, float offset, float octaves, float octavesinner, float scale, float focus) {
  float r = length(vec2(_st.y - 0.5, _st.x - 0.5));
  float foc = pow(r, abs(focus));
  int oin = int(abs(octavesinner));
  float fri = fract(octavesinner);
  float fbmx = 0.0;
  {
    vec2 pos = scalei * _st;
    float sc = 1.0;
    for (int io = 0; io < 8; io++) {
      fbmx += sc * _noise(vec3(pos, offset * iTime));
      pos *= 2.0;
      sc /= 2.0;
      if (io >= oin) break;
    }
    fbmx += fri * sc * _noise(vec3(pos, offset * iTime));
    fbmx = (0.5 + 0.5 * fbmx) - foc;
  }
  float fbmy = 0.0;
  {
    vec2 pos = scalei * (_st + vec2(5.123, 3.987));
    float sc = 1.0;
    for (int io = 0; io < 8; io++) {
      fbmy += sc * _noise(vec3(pos, offset * iTime));
      pos *= 2.0;
      sc /= 2.0;
      if (io >= oin) break;
    }
    fbmy += fri * sc * _noise(vec3(pos, offset * iTime));
    fbmy = (0.5 + 0.5 * fbmy) - foc;
  }
  int on = int(abs(octaves));
  float fr = fract(octaves);
  float fbm = 0.0;
  vec2 pos = scale * vec2(fbmx, fbmy);
  float sc = 1.0;
  for (int io = 0; io < 8; io++) {
    fbm += sc * _noise(vec3(pos, offset * iTime));
    pos *= 2.0;
    sc /= 2.0;
    if (io >= on) break;
  }
  fbm += fr * sc * _noise(vec3(pos, offset * iTime));
  fbm = (0.5 + 0.5 * fbm) - foc;
  return vec4(fbm, fbm, fbm, 1.0);
}

// ncontour, from metagrowing/extra-shaders-for-hydra (lib/lib-noise.js),
// AGPL-3.0, by Thomas Jourdan.
vec4 ncontour(vec2 _st, float thresh, float smoothAmt, float octaves, float scale, float speed, float step) {
  vec2 st = _st - 0.5;
  float sc = scale;
  float sp = speed;
  float d0 = _noise(vec3(st * sc, sp * iTime));
  for (int ni = 1; ni < 5; ++ni) {
    if (ni >= int(octaves)) break;
    sp /= step;
    sc *= step;
    d0 += _noise(vec3(st * sc, sp * iTime));
  }
  float d = distance(d0, thresh);
  float g = smoothstep(0.0, smoothAmt, d);
  return vec4(vec3(g, g, g), 1.0);
}

// pulse, from metagrowing/extra-shaders-for-hydra (lib/lib-pattern.js),
// AGPL-3.0, by Thomas Jourdan.
vec4 pulse(vec2 _st, float edge, float width, float epsilon) {
  float ea = abs(epsilon);
  float wa = abs(width);
  float d0 = smoothstep(edge - ea, edge + ea, _st.x);
  float d1 = smoothstep(edge + wa - ea, edge + wa + ea, _st.x);
  float d = d0 - d1;
  return vec4(d, d, d, 1.0);
}

// pulsetrain, from metagrowing/extra-shaders-for-hydra (lib/lib-pattern.js),
// AGPL-3.0, by Thomas Jourdan.
vec4 pulsetrain(vec2 _st, float train, float edge, float width, float epsilon) {
  float ea = abs(epsilon);
  float wa = abs(width);
  float xp = _st.x;
  float d = 0.0;
  int itr = int(train);
  for (int ii = 0; ii < 10; ii++) {
    float d0 = smoothstep(edge - ea, edge + ea, xp);
    float d1 = smoothstep(edge + wa - ea, edge + wa + ea, xp);
    if (ii >= itr) break;
    d += d0 - d1;
    xp += 1.0 / float(itr);
  }
  return vec4(d, d, d, 1.0);
}

// hextile, from metagrowing/extra-shaders-for-hydra (lib/lib-pattern.js),
// AGPL-3.0, by Thomas Jourdan.
vec4 hextile(vec2 _st, float tiles) {
  const vec2 hs = vec2(1.0, 1.7320508075688772);
  vec2 p = (_st - 0.5) * tiles;
  vec4 hC = floor(vec4(p, p - vec2(0.5, 1.0)) / hs.xyxy) + 0.5;
  vec4 h = vec4(p - hC.xy * hs, p - (hC.zw + 0.5) * hs);
  float d = length(h.xy) < length(h.zw) ?
            (fract(p.x * 0.5) < 0.5 ? 0.75 : 0.0) :
            (fract(p.x * 0.5 - hs.y - 0.01) < 0.5 ? 0.25 : 1.0);
  return vec4(d, d, d, 1.0);
}

// concentric, from metagrowing/extra-shaders-for-hydra (lib/lib-pattern.js),
// AGPL-3.0, by Thomas Jourdan.
vec4 concentric(vec2 _st, float scale, float centerX, float centerY) {
  float d = sin(scale * distance(_st, vec2(centerX, centerY)));
  return vec4(d, d, d, 1.0);
}

// brick, from metagrowing/extra-shaders-for-hydra (lib/lib-pattern.js),
// AGPL-3.0, by Thomas Jourdan (see Darwyn Peachey, Building Procedural
// Textures, page 37).
vec4 brick(vec2 _st, float width, float height, float gap) {
  vec2 p = _st - 0.5;
  const float eps = 0.001;
  float bmwidth = width + gap;
  float bmheight = height + gap;
  float mwf = gap * 0.5 / bmwidth;
  float mhf = gap * 0.5 / bmheight;
  float bms = p.x / bmwidth;
  float bmt = p.y / bmheight;
  if (mod(bmt * 0.5, 1.0) > 0.5) bms += 0.5;
  float sbrick = floor(bms);
  float tbrick = floor(bmt);
  bms -= sbrick;
  bmt -= tbrick;
  float w = smoothstep(mwf, mwf + eps, bms) - smoothstep(1.0 - mwf - eps, 1.0 - mwf, bms);
  float h = smoothstep(mhf, mhf + eps, bmt) - smoothstep(1.0 - mhf - eps, 1.0 - mhf, bmt);
  float d = w * h;
  return vec4(d, d, d, 1.0);
}

// wave, from metagrowing/extra-shaders-for-hydra (lib/lib-pattern.js),
// AGPL-3.0, by Thomas Jourdan.
vec4 wave(vec2 _st, float waveTime, float frequ, float loops, float thick) {
  const float eps = 0.001;
  float x = _st.x - waveTime;
  float y = _st.y - 0.5;
  float sc = 0.25;
  float fr = frequ;
  float l = 0.0;
  for (int i = 0; i < 6; ++i) {
    y += sc * sin(fr * x);
    if (l >= loops) break;
    sc *= 0.5;
    fr *= 2.0;
    l += 1.0;
  }
  float d = smoothstep(0.0, eps, y) - smoothstep(thick, thick + eps, y);
  return vec4(d, d, d, 1.0);
}

// lissa, from metagrowing/extra-shaders-for-hydra (lib/lib-pattern.js),
// AGPL-3.0, by Thomas Jourdan (see https://en.wikipedia.org/wiki/Harmonograph).
vec4 lissa(vec2 _st, float lissaTime, float frequ, float loops, float thick) {
  const float eps = 0.001;
  vec2 st2 = _st - 0.5;
  vec2 pol = vec2(atan(st2.y, st2.x), 2.0 * length(st2));
  float x = pol.x - lissaTime;
  float y = pol.y - 0.5;
  float sc = 0.25;
  float fr = frequ;
  float l = 0.0;
  for (int i = 0; i < 6; ++i) {
    y += sc * sin(fr * x);
    if (l >= loops) break;
    sc *= 0.5;
    fr *= 2.0;
    l += 1.0;
  }
  float d = smoothstep(0.0, eps, y) - smoothstep(thick, thick + eps, y);
  return vec4(d, d, d, 1.0);
}

// mirrorX/mirrorY/mirrorX2/mirrorY2/mirrorWrap, from geikha/hyper-hydra
// (hydra-fractals.js), MIT license.
vec2 mirrorX(vec2 _st, float pos, float coverage) {
  _st.x = (0.0 - abs(fract(_st.x / coverage) - (1.0 - 0.5 - pos)) + 0.5 - pos) * coverage;
  return _st;
}

vec2 mirrorY(vec2 _st, float pos, float coverage) {
  _st.y = (0.0 - abs(fract(_st.y / coverage) - (1.0 - 0.5 - pos)) + 0.5 - pos) * coverage;
  return _st;
}

vec2 mirrorX2(vec2 _st, float pos, float coverage) {
  _st.x = (abs(fract(_st.x / coverage) - (1.0 - 0.5 - pos)) + 0.5 - pos) * coverage;
  return _st;
}

vec2 mirrorY2(vec2 _st, float pos, float coverage) {
  _st.y = (0.0 - abs(fract(_st.y / coverage) - (1.0 - 0.5 - pos)) + 0.5 - pos) * coverage;
  return _st;
}

vec2 mirrorWrap(vec2 _st) {
  return -abs(fract(_st / 2.0) * 2.0 - 1.0) + 1.0;
}

// mandeloffs, from geikha/hyper-hydra (hydra-fractals.js), MIT license -
// offsets `_st` by one step of a Mandelbrot iteration (z -> z^2 + c).
vec2 mandeloffs(vec2 _st, float amt, float offx, float offy) {
  vec2 scaled = _st * 16.0 - vec2(8.0, 8.0);
  vec2 np = vec2(scaled.x * scaled.x - scaled.y * scaled.y,
                 2.0 * scaled.x * scaled.y) + vec2(offx, offy);
  vec2 diff = (np - scaled) * amt;
  return _st + diff;
}
