/* BẢN CHÉP từ zalo-rs/static/zalo/wallpaper-art.js — sửa ở Zalo rồi chép sang đây (kho hình nền chung Zalo ↔ Tele).
   Bộ sưu tập hình nền MẪU — minh hoạ vẽ bằng SVG (gốc, không dùng ảnh có bản quyền), tạo sinh theo hạt giống nên luôn giống nhau.
   Mỗi mẫu có mã `s01`…`s30` (khớp `crate::wallpaper::PRESETS` ở Rust). Dùng chung với Tele. Nạp TRƯỚC wallpaper.js. */
(function (global) {
  "use strict";
  var W = 400, H = 700;
  function rng(seed) { var s = seed >>> 0 || 1; return function () { s ^= s << 13; s >>>= 0; s ^= s >> 17; s ^= s << 5; s >>>= 0; return s / 4294967296; }; }
  function wrap(defs, body) { return '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ' + W + " " + H + '" preserveAspectRatio="xMidYMid slice"><defs>' + defs + "</defs>" + body + "</svg>"; }
  function lin(id, a, b, c) { return '<linearGradient id="' + id + '" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="' + a + '"/>' + (c ? '<stop offset=".55" stop-color="' + b + '"/><stop offset="1" stop-color="' + c + '"/>' : '<stop offset="1" stop-color="' + b + '"/>') + "</linearGradient>"; }
  function rad(id, c, o) { return '<radialGradient id="' + id + '"><stop offset="0" stop-color="' + c + '" stop-opacity="' + (o || 1) + '"/><stop offset="1" stop-color="' + c + '" stop-opacity="0"/></radialGradient>'; }

  // 1) Núi nhiều lớp + mặt trời/trăng
  function mountains(p, seed) {
    var r = rng(seed), body = '<rect width="400" height="700" fill="url(#sky)"/><circle cx="' + (80 + r() * 240) + '" cy="' + (150 + r() * 90) + '" r="' + (34 + r() * 22) + '" fill="' + p.sun + '" opacity=".9"/>';
    for (var l = 0; l < 4; l++) {
      var y = 330 + l * 80, pts = "0,700 0," + y; for (var x = 0; x <= 400; x += 50) pts += " " + x + "," + (y - 40 - r() * (70 - l * 8)); pts += " 400,700";
      body += '<polygon points="' + pts + '" fill="' + p.ridge[l] + '" opacity="' + (0.55 + l * 0.15) + '"/>';
    }
    return wrap(lin("sky", p.a, p.b, p.c), body);
  }
  // 2) Biển + sóng
  function sea(p, seed) {
    var r = rng(seed), body = '<rect width="400" height="700" fill="url(#sky)"/><circle cx="' + (120 + r() * 160) + '" cy="300" r="46" fill="' + p.sun + '" opacity=".85"/><rect y="380" width="400" height="320" fill="' + p.water + '"/>';
    for (var i = 0; i < 9; i++) { var y = 400 + i * 34, a = 8 + r() * 10; body += '<path d="M-20 ' + y + " Q 50 " + (y - a) + " 100 " + y + " T 220 " + y + " T 340 " + y + " T 460 " + y + '" fill="none" stroke="' + p.foam + '" stroke-opacity="' + (0.15 + i * 0.05) + '" stroke-width="3"/>'; }
    return wrap(lin("sky", p.a, p.b), body);
  }
  // 3) Rừng thông
  function forest(p, seed) {
    var r = rng(seed), body = '<rect width="400" height="700" fill="url(#sky)"/>';
    for (var l = 0; l < 3; l++) for (var i = 0; i < 9 + l * 2; i++) {
      var x = r() * 420 - 10, base = 460 + l * 90 + r() * 30, h = 110 + r() * 90 - l * 15, w = 40 + r() * 25;
      body += '<polygon points="' + x + "," + (base - h) + " " + (x - w / 2) + "," + base + " " + (x + w / 2) + "," + base + '" fill="' + p.tree[l] + '"/>';
    }
    return wrap(lin("sky", p.a, p.b), body + '<rect y="640" width="400" height="60" fill="' + p.tree[2] + '"/>');
  }
  // 4) Trăng sao
  function night(p, seed) {
    var r = rng(seed), body = '<rect width="400" height="700" fill="url(#sky)"/>';
    for (var i = 0; i < 70; i++) body += '<circle cx="' + r() * 400 + '" cy="' + r() * 520 + '" r="' + (0.6 + r() * 1.6) + '" fill="#fff" opacity="' + (0.3 + r() * 0.7) + '"/>';
    var mx = 250 + r() * 80;
    body += '<mask id="mk"><rect width="400" height="700" fill="#fff"/><circle cx="' + (mx + 22) + '" cy="138" r="44" fill="#000"/></mask><circle cx="' + mx + '" cy="150" r="48" fill="' + p.moon + '" mask="url(#mk)"/>';
    body += '<path d="M0 600 Q 100 520 200 580 T 400 560 V700 H0 Z" fill="' + p.hill + '"/>';
    return wrap(lin("sky", p.a, p.b, p.c), body);
  }
  // 5) Cồn cát
  function dunes(p, seed) {
    var r = rng(seed), body = '<rect width="400" height="700" fill="url(#sky)"/><circle cx="' + (60 + r() * 280) + '" cy="200" r="50" fill="' + p.sun + '" opacity=".9"/>';
    for (var l = 0; l < 4; l++) { var y = 380 + l * 80, a = 40 + r() * 40; body += '<path d="M0 ' + y + " Q 120 " + (y - a) + " 240 " + (y + 10) + " T 400 " + (y - 10) + ' V700 H0 Z" fill="' + p.sand[l] + '"/>'; }
    return wrap(lin("sky", p.a, p.b), body);
  }
  // 6) Mưa nhẹ
  function rain(p, seed) {
    var r = rng(seed), body = '<rect width="400" height="700" fill="url(#sky)"/>';
    for (var i = 0; i < 90; i++) { var x = r() * 420, y = r() * 700, l = 14 + r() * 22; body += '<line x1="' + x + '" y1="' + y + '" x2="' + (x - 5) + '" y2="' + (y + l) + '" stroke="' + p.drop + '" stroke-opacity="' + (0.2 + r() * 0.4) + '" stroke-width="1.4"/>'; }
    return wrap(lin("sky", p.a, p.b), body);
  }
  // 7) Bokeh
  function bokeh(p, seed) {
    var r = rng(seed), body = '<rect width="400" height="700" fill="url(#sky)"/>';
    for (var i = 0; i < 26; i++) body += '<circle cx="' + r() * 400 + '" cy="' + r() * 700 + '" r="' + (14 + r() * 46) + '" fill="' + p.dots[i % p.dots.length] + '" opacity="' + (0.12 + r() * 0.28) + '"/>';
    return wrap(lin("sky", p.a, p.b), body);
  }
  // 8) Đốm màu loang
  function blobs(p, seed) {
    var r = rng(seed), body = '<rect width="400" height="700" fill="' + p.a + '"/>';
    for (var i = 0; i < 5; i++) body += '<circle cx="' + r() * 400 + '" cy="' + r() * 700 + '" r="' + (140 + r() * 160) + '" fill="url(#b' + i + ')"/>';
    var defs = ""; for (var j = 0; j < 5; j++) defs += rad("b" + j, p.blobs[j % p.blobs.length], 0.9);
    return wrap(defs, body);
  }
  // 9) Hình học tam giác
  function triangles(p, seed) {
    var r = rng(seed), body = '<rect width="400" height="700" fill="' + p.a + '"/>';
    for (var y = 0; y < 700; y += 70) for (var x = 0; x < 400; x += 70) {
      var c = p.tri[Math.floor(r() * p.tri.length)];
      body += '<polygon points="' + x + "," + y + " " + (x + 70) + "," + y + " " + x + "," + (y + 70) + '" fill="' + c + '" opacity="' + (0.25 + r() * 0.5) + '"/><polygon points="' + (x + 70) + "," + y + " " + (x + 70) + "," + (y + 70) + " " + x + "," + (y + 70) + '" fill="' + p.tri[Math.floor(r() * p.tri.length)] + '" opacity="' + (0.2 + r() * 0.5) + '"/>';
    }
    return wrap("", body);
  }
  // 10) Hoa năm cánh (đào, mai…)
  function flowers(p, seed) {
    var r = rng(seed), body = '<rect width="400" height="700" fill="url(#sky)"/>';
    for (var b = 0; b < 4; b++) { var x0 = r() * 400, y0 = 700; body += '<path d="M' + x0 + " " + y0 + " Q " + (x0 + (r() * 120 - 60)) + " " + (y0 - 300) + " " + (r() * 400) + " " + (r() * 300) + '" fill="none" stroke="' + p.branch + '" stroke-width="' + (3 + r() * 3) + '" stroke-linecap="round" opacity=".8"/>'; }
    for (var i = 0; i < 24; i++) {
      var cx = r() * 400, cy = r() * 700, s = 10 + r() * 22, rot = r() * 72, g = '<g transform="translate(' + cx + " " + cy + ") rotate(" + rot + ')">';
      for (var k = 0; k < 5; k++) g += '<ellipse cx="0" cy="' + (-s * 0.75) + '" rx="' + (s * 0.5) + '" ry="' + s * 0.8 + '" fill="' + p.petal + '" transform="rotate(' + k * 72 + ')" opacity=".92"/>';
      body += g + '<circle r="' + s * 0.22 + '" fill="' + p.heart + '"/></g>';
    }
    return wrap(lin("sky", p.a, p.b), body);
  }
  // 11) Lá cây nhiệt đới
  function leaves(p, seed) {
    var r = rng(seed), body = '<rect width="400" height="700" fill="url(#sky)"/>';
    for (var i = 0; i < 16; i++) {
      var cx = r() * 400, cy = r() * 700, s = 50 + r() * 90, rot = r() * 360;
      body += '<g transform="translate(' + cx + " " + cy + ") rotate(" + rot + ')"><ellipse rx="' + s * 0.28 + '" ry="' + s + '" fill="' + p.leaf[i % p.leaf.length] + '" opacity="' + (0.35 + r() * 0.4) + '"/><line y1="' + -s + '" y2="' + s + '" stroke="' + p.vein + '" stroke-opacity=".35" stroke-width="2"/></g>';
    }
    return wrap(lin("sky", p.a, p.b), body);
  }
  // 12) Lồng đèn / bao lì xì (Tết)
  function lanterns(p, seed) {
    var r = rng(seed), body = '<rect width="400" height="700" fill="url(#sky)"/>';
    for (var i = 0; i < 9; i++) {
      var x = 30 + r() * 340, top = r() * 260, len = 60 + r() * 160, s = 22 + r() * 18;
      body += '<line x1="' + x + '" y1="0" x2="' + x + '" y2="' + (top + len) + '" stroke="' + p.gold + '" stroke-opacity=".7"/><ellipse cx="' + x + '" cy="' + (top + len + s) + '" rx="' + s + '" ry="' + s * 1.15 + '" fill="' + p.red + '"/><rect x="' + (x - s * 0.5) + '" y="' + (top + len - 2) + '" width="' + s + '" height="6" fill="' + p.gold + '"/><line x1="' + x + '" y1="' + (top + len + s * 2.2) + '" x2="' + x + '" y2="' + (top + len + s * 3.3) + '" stroke="' + p.gold + '" stroke-width="3"/>';
    }
    for (var k = 0; k < 40; k++) body += '<circle cx="' + r() * 400 + '" cy="' + r() * 700 + '" r="' + (1 + r() * 2) + '" fill="' + p.gold + '" opacity=".6"/>';
    return wrap(lin("sky", p.a, p.b), body);
  }
  // 13) Bánh chưng / ô vuông vàng
  function squares(p, seed) {
    var r = rng(seed), body = '<rect width="400" height="700" fill="' + p.a + '"/>';
    for (var i = 0; i < 22; i++) {
      var x = r() * 380, y = r() * 680, s = 50 + r() * 60, rot = r() * 90 - 45;
      body += '<g transform="translate(' + x + " " + y + ") rotate(" + rot + ')"><rect width="' + s + '" height="' + s + '" rx="6" fill="' + p.sq[i % p.sq.length] + '" opacity=".75"/><path d="M0 ' + s / 2 + " H" + s + " M" + s / 2 + " 0 V" + s + '" stroke="' + p.gold + '" stroke-width="2" opacity=".8"/></g>';
    }
    return wrap("", body);
  }
  // 14) Gợn sóng đồng tâm
  function ripples(p, seed) {
    var r = rng(seed), body = '<rect width="400" height="700" fill="url(#sky)"/>';
    for (var c = 0; c < 3; c++) { var cx = r() * 400, cy = r() * 700; for (var i = 1; i < 9; i++) body += '<circle cx="' + cx + '" cy="' + cy + '" r="' + i * 38 + '" fill="none" stroke="' + p.ring + '" stroke-opacity="' + (0.5 / i) + '" stroke-width="2"/>'; }
    return wrap(lin("sky", p.a, p.b), body);
  }

  var SCENES = [
    ["s01", "Núi hoàng hôn", mountains, { a: "#ffb88c", b: "#de6262", c: "#5b2a5b", sun: "#fff3c4", ridge: ["#8a4f7d", "#6a3a6f", "#4a2a5f", "#2c1a47"] }, 11],
    ["s02", "Núi sương mai", mountains, { a: "#cfe8ff", b: "#a6c8f0", sun: "#ffffff", ridge: ["#9db8d8", "#7d9cc4", "#5f7fae", "#3f5f90"] }, 12],
    ["s03", "Núi tuyết hồng", mountains, { a: "#fbd3e9", b: "#bb377d", sun: "#fff", ridge: ["#d17ba6", "#a2457c", "#7b2a63", "#4a1946"] }, 13],
    ["s04", "Biển xanh", sea, { a: "#74c0fc", b: "#e3f6ff", sun: "#fff6bf", water: "#2a82c7", foam: "#ffffff" }, 21],
    ["s05", "Biển chiều", sea, { a: "#fda085", b: "#f6d365", sun: "#fff", water: "#e0795b", foam: "#fff1d6" }, 22],
    ["s06", "Rừng thông", forest, { a: "#d7f5df", b: "#9ad6a9", tree: ["#5fa879", "#3f8a5c", "#26664a"] }, 31],
    ["s07", "Rừng đêm", forest, { a: "#1f3a4d", b: "#35687a", tree: ["#27566a", "#1a3f55", "#0f2a3c"] }, 32],
    ["s08", "Trăng sao", night, { a: "#0b1026", b: "#1c2a5a", c: "#3a2c6e", moon: "#fff7d6", hill: "#10152e" }, 41],
    ["s09", "Đêm tím", night, { a: "#1a0f3c", b: "#4a2c7a", c: "#8e4a9c", moon: "#ffe9f4", hill: "#241048" }, 42],
    ["s10", "Cồn cát vàng", dunes, { a: "#ffe9b0", b: "#ffcf86", sun: "#fff", sand: ["#f2c66d", "#e7ad58", "#d9944a", "#c27a3d"] }, 51],
    ["s11", "Cồn cát hồng", dunes, { a: "#ffd7e2", b: "#ffb3c8", sun: "#fff", sand: ["#f1a6bd", "#e58aa8", "#d46c92", "#b9507a"] }, 52],
    ["s12", "Mưa xanh", rain, { a: "#3d5a73", b: "#6f93ad", drop: "#e8f4ff" }, 61],
    ["s13", "Mưa đêm", rain, { a: "#141a2b", b: "#2f3b57", drop: "#b6c8ff" }, 62],
    ["s14", "Bokeh ấm", bokeh, { a: "#40203a", b: "#a14a3d", dots: ["#ffd479", "#ff9a76", "#ffb3c1", "#fff1a8"] }, 71],
    ["s15", "Bokeh lạnh", bokeh, { a: "#0f2a4a", b: "#2f6a9a", dots: ["#8fd3ff", "#b9f2ff", "#d6e4ff", "#7aa7ff"] }, 72],
    ["s16", "Loang cầu vồng", blobs, { a: "#fdf2ff", blobs: ["#ff9ad5", "#9ad5ff", "#fff29a", "#b99aff", "#9affc5"] }, 81],
    ["s17", "Loang đêm", blobs, { a: "#0d0f1f", blobs: ["#5b3df5", "#d94bd1", "#22b6ff", "#ff6b9a", "#3fe0b4"] }, 82],
    ["s18", "Tam giác pastel", triangles, { a: "#fbf6ff", tri: ["#cdb4ff", "#ffc8dd", "#bde0fe", "#fdffb6", "#caffbf"] }, 91],
    ["s19", "Tam giác đêm", triangles, { a: "#10142a", tri: ["#3a3d7a", "#6247aa", "#2a6f97", "#a06cd5", "#1b4965"] }, 92],
    ["s20", "Hoa đào", flowers, { a: "#fff1f4", b: "#ffd9e2", branch: "#7a4b3a", petal: "#ff8fab", heart: "#ffd166" }, 101],
    ["s21", "Hoa mai", flowers, { a: "#fffbe8", b: "#fff0b0", branch: "#6b4a2d", petal: "#ffc93c", heart: "#d9480f" }, 102],
    ["s22", "Hoa tím", flowers, { a: "#f3ecff", b: "#d9c8ff", branch: "#5b4a7a", petal: "#a78bfa", heart: "#fde68a" }, 103],
    ["s23", "Lá nhiệt đới", leaves, { a: "#e6f9e8", b: "#b7e8bf", leaf: ["#2f9e5b", "#52b788", "#1b7a49", "#74c69d"], vein: "#0b3d2e" }, 111],
    ["s24", "Lá đêm", leaves, { a: "#0d1f1a", b: "#17382e", leaf: ["#2d6a4f", "#40916c", "#1b4332", "#52b788"], vein: "#d8f3dc" }, 112],
    ["s25", "Lồng đèn đỏ", lanterns, { a: "#7a0f1f", b: "#c1272d", gold: "#ffd166", red: "#e63946" }, 121],
    ["s26", "Lồng đèn đêm", lanterns, { a: "#1d0b2e", b: "#4a1942", gold: "#ffd166", red: "#ef476f" }, 122],
    ["s27", "Bánh chưng", squares, { a: "#d9f2c8", gold: "#e0b43c", sq: ["#4f9d4f", "#3d8b46", "#68b56a", "#2f7a3f"] }, 131],
    ["s28", "Lì xì vàng", squares, { a: "#b3172b", gold: "#ffd166", sq: ["#e63946", "#d62839", "#f0524f", "#c1121f"] }, 132],
    ["s29", "Gợn nước", ripples, { a: "#cdeffd", b: "#7fc8f0", ring: "#ffffff" }, 141],
    ["s30", "Gợn đêm", ripples, { a: "#0a1030", b: "#2a3a8a", ring: "#9fb4ff" }, 142]
  ];
  var SAMPLES = SCENES.map(function (s) { return { id: s[0], label: s[1], svg: s[2](s[3], s[4]) }; });
  function url(sample) { return 'url("data:image/svg+xml;utf8,' + encodeURIComponent(sample.svg) + '")'; }
  global.WallpaperArt = { SAMPLES: SAMPLES, css: function (id) { var s = SAMPLES.filter(function (x) { return x.id === id; })[0]; return s ? url(s) + " center/cover no-repeat" : ""; } };
})(typeof window !== "undefined" ? window : globalThis);
