/* Đồng bộ nguyên văn từ zalo-rs (static/zalo/emoji.js) — mô-đun dùng chung Zalo/Tele; sửa ở một nơi thì chép sang nơi kia. */
/* Bảng emoji và thanh cảm xúc kiểu Tele — MÔ-ĐUN DÙNG CHUNG (không phụ thuộc app.js). Nạp SAU emoji-data.js.
   - Emoji.open({anchor,onPick,onClose,keepOpen}): bảng đầy đủ — ô tìm kiếm (tên Anh + từ khoá Việt, không phân biệt dấu), thanh nhóm, «Dùng gần đây», cuộn có tiêu đề nhóm, phím mũi tên.
   - Emoji.reactionBar({quick,all,mine,onPick}): hàng cảm xúc nhanh + nút «⌄» mở toàn bộ danh sách (mỗi mục {key,emoji,label}). */
(function (global) {
  "use strict";
  var RECENT_KEY = "emoji-recent", RECENT_MAX = 32, COLS = 8;
  var strip = function (s) { return String(s || "").toLowerCase().normalize("NFD").replace(/[̀-ͯ]/g, "").replace(/đ/g, "d"); };
  function data() { return (global.EmojiData && global.EmojiData.categories) || []; }
  // Kho lưu «dùng gần đây» cắm được: mặc định localStorage (Zalo); Tele cắm kho dùng cookie (Tele cấm localStorage). get()→chuỗi JSON|null, set(chuỗi JSON).
  var store = {
    get: function () { try { return localStorage.getItem(RECENT_KEY); } catch (e) { return null; } },
    set: function (v) { try { localStorage.setItem(RECENT_KEY, v); } catch (e) { /* riêng tư */ } }
  };
  function setStorage(s) { if (s && typeof s.get === "function" && typeof s.set === "function") store = s; }
  function recent() { try { var a = JSON.parse(store.get() || "[]"); return Array.isArray(a) ? a.filter(function (x) { return typeof x === "string"; }).slice(0, RECENT_MAX) : []; } catch (e) { return []; } }
  function remember(ch) { try { var a = recent().filter(function (x) { return x !== ch; }); a.unshift(ch); store.set(JSON.stringify(a.slice(0, RECENT_MAX))); } catch (e) { /* riêng tư */ } }
  function index() {
    var out = []; data().forEach(function (c) { c.items.forEach(function (it) { out.push({ ch: it[0], name: it[1], vi: it[2], cat: c.id, hay: strip(it[1] + " " + it[2]) }); }); }); return out;
  }
  /** Tìm: mọi từ trong truy vấn phải xuất hiện (tên Anh hoặc từ khoá Việt); sắp xếp: khớp đầu từ trước. */
  function search(query, idx) {
    var words = strip(query).split(/\s+/).filter(Boolean); if (!words.length) return [];
    idx = idx || index();
    return idx.filter(function (e) { return words.every(function (w) { return e.hay.indexOf(w) >= 0; }); })
      .sort(function (a, b) { var s = function (e) { return words.reduce(function (n, w) { return n + (e.hay.indexOf(" " + w) >= 0 || e.hay.indexOf(w) === 0 ? 0 : 1); }, 0); }; return s(a) - s(b); });
  }
  function el(tag, attrs) {
    var n = document.createElement(tag), kids = Array.prototype.slice.call(arguments, 2);
    Object.keys(attrs || {}).forEach(function (k) { if (k === "class") n.className = attrs[k]; else if (k.slice(0, 2) === "on") n.addEventListener(k.slice(2), attrs[k]); else if (attrs[k] !== null && attrs[k] !== undefined && attrs[k] !== false) n.setAttribute(k, attrs[k]); });
    kids.flat().forEach(function (k) { if (k !== null && k !== undefined) n.append(k); });
    return n;
  }
  function open(opts) {
    opts = opts || {};
    var old = document.getElementById("emoji-panel"); if (old) { if (old.__close) old.__close(); else old.remove(); }      // đóng ĐÚNG cách (gỡ bộ lắng nghe phím/chuột, báo onClose) trước khi mở bảng mới
    var idx = index(), recents = recent(), cats = data();
    var input = el("input", { type: "search", class: "ep-q", placeholder: "Tìm emoji…", "aria-label": "Tìm emoji", autocomplete: "off" });
    var scroll = el("div", { class: "ep-scroll", tabindex: "-1" }), tabsBar = el("div", { class: "ep-tabs", role: "tablist", "aria-label": "Nhóm emoji" });
    var sections = {};
    function cell(ch, name) { return el("button", { type: "button", class: "ep-e", "aria-label": name, title: name, onclick: function () { pick(ch); } }, ch); }
    function pick(ch) { remember(ch); if (opts.onPick) opts.onPick(ch); if (!opts.keepOpen) close(); else if (!input.value) drawAll(); }
    function drawAll() {
      sections = {}; var kids = []; recents = recent(); var meta = [];
      if (recents.length) { kids.push(sec("recent", "Dùng gần đây", recents.map(function (c) { return cell(c, c); }))); meta.push(["recent", "🕘", "Dùng gần đây"]); }
      cats.forEach(function (c) { if (c.items.length) { kids.push(sec(c.id, c.label, c.items.map(function (it) { return cell(it[0], it[2] ? it[2].split(" ").slice(0, 3).join(" ") + " — " + it[1] : it[1]); }))); meta.push([c.id, c.icon, c.label]); } });
      scroll.replaceChildren.apply(scroll, kids);
      tabsBar.replaceChildren.apply(tabsBar, meta.map(function (m, i) { return el("button", { type: "button", role: "tab", class: "ep-t" + (i === 0 ? " on" : ""), "data-cat": m[0], "aria-label": m[2], title: m[2], onclick: function () { go(m[0]); } }, m[1]); }));
    }
    function sec(id, label, cells) { var h = el("h5", { class: "ep-h", id: "ep-" + id }, label), g = el("div", { class: "ep-g" }, cells), w = el("section", {}, h, g); sections[id] = w; return w; }
    function go(id) { var s = sections[id]; if (s) scroll.scrollTop = s.offsetTop; }       // .ep-scroll là phần tử định vị ⇒ offsetTop của nhóm tính ngay từ đỉnh vùng cuộn
    function spy() {
      var top = scroll.scrollTop + 4, cur = null; Object.keys(sections).forEach(function (id) { if (sections[id].offsetTop <= top) cur = id; });
      Array.prototype.forEach.call(tabsBar.children, function (b) { b.classList.toggle("on", b.getAttribute("data-cat") === cur); });
    }
    scroll.addEventListener("scroll", function () { if (!input.value) spy(); });
    input.addEventListener("input", function () {
      var q = input.value.trim(); tabsBar.hidden = !!q;
      if (!q) return drawAll();
      var res = search(q, idx);
      scroll.replaceChildren(res.length ? el("section", {}, el("h5", { class: "ep-h" }, "Kết quả (" + res.length + ")"), el("div", { class: "ep-g" }, res.slice(0, 160).map(function (e) { return cell(e.ch, e.vi ? e.vi + " — " + e.name : e.name); })))
        : el("p", { class: "ep-none", role: "status" }, "Không thấy emoji nào khớp “" + q + "”."));
    });
    function onKey(e) {
      if (e.key === "Escape") { e.stopPropagation(); close(); return; }
      var cur = document.activeElement; if (!cur || !cur.classList.contains("ep-e")) { if (e.key === "ArrowDown" && cur === input) { var f = panel.querySelector(".ep-e"); if (f) { f.focus(); e.preventDefault(); } } return; }
      var all = Array.prototype.slice.call(panel.querySelectorAll(".ep-e")), i = all.indexOf(cur), d = ({ ArrowRight: 1, ArrowLeft: -1, ArrowDown: COLS, ArrowUp: -COLS })[e.key];
      if (d === undefined) return; e.preventDefault();
      if (e.key === "ArrowUp" && i < COLS) { input.focus(); return; }
      var next = all[Math.max(0, Math.min(all.length - 1, i + d))]; if (next) { next.focus(); next.scrollIntoView({ block: "nearest" }); }
    }
    function onDown(e) { if (!panel.contains(e.target) && !(opts.anchor && opts.anchor.contains(e.target))) close(); }
    function close() { if (!panel.isConnected) return; panel.remove(); document.removeEventListener("keydown", onKey, true); document.removeEventListener("mousedown", onDown, true); if (opts.onClose) opts.onClose(); }
    var panel = el("div", { class: "ep", id: "emoji-panel", role: "dialog", "aria-label": "Chọn emoji" }, input, tabsBar, scroll);
    panel.__close = close;
    document.body.append(panel); drawAll();
    if (opts.anchor) {                                    // đặt phía trên nút, cạnh phải thẳng hàng; thiếu chỗ thì xuống dưới
      var r = opts.anchor.getBoundingClientRect(), w = panel.offsetWidth, h = panel.offsetHeight;
      panel.style.left = Math.max(8, Math.min(r.right - w, window.innerWidth - w - 8)) + "px";
      panel.style.top = Math.max(8, r.top - h - 8 >= 8 ? r.top - h - 8 : Math.min(r.bottom + 8, window.innerHeight - h - 8)) + "px";
    }
    document.addEventListener("keydown", onKey, true); setTimeout(function () { document.addEventListener("mousedown", onDown, true); }, 0);
    input.focus();
    return { close: close, panel: panel };
  }
  /** Thanh cảm xúc: quick = [{key,emoji,label}] hiện sẵn; all = toàn bộ (mở bằng «⌄»); mine = key cảm xúc mình đã thả (làm nổi). */
  function reactionBar(opts) {
    var wrap = el("div", { class: "rx" }), row = el("div", { class: "rx-row", role: "group", "aria-label": "Thả cảm xúc" }), full = el("div", { class: "rx-all", hidden: true, role: "group", "aria-label": "Tất cả cảm xúc" });
    function btn(r) { return el("button", { type: "button", class: "rx-b" + (opts.mine && opts.mine === r.key ? " mine" : ""), "data-key": r.key, "aria-label": r.label, title: r.label, onclick: function () { if (opts.onPick) opts.onPick(r.key, r); } }, r.emoji); }
    function chevron() {      // SVG dựng bằng DOM (không innerHTML) thay ký tự «⌄» vốn thiếu trong nhiều phông
      var NS = "http://www.w3.org/2000/svg", svg = document.createElementNS(NS, "svg"), path = document.createElementNS(NS, "path");
      svg.setAttribute("viewBox", "0 0 24 24"); svg.setAttribute("width", "18"); svg.setAttribute("height", "18"); svg.setAttribute("fill", "none"); svg.setAttribute("stroke", "currentColor");
      svg.setAttribute("stroke-width", "2.4"); svg.setAttribute("stroke-linecap", "round"); svg.setAttribute("stroke-linejoin", "round"); svg.setAttribute("aria-hidden", "true");
      path.setAttribute("d", "m6 9 6 6 6-6"); svg.appendChild(path); return svg;
    }
    var more = el("button", { type: "button", class: "rx-more", "aria-expanded": "false", "aria-label": "Xem thêm cảm xúc", title: "Xem thêm cảm xúc", onclick: function () {
      full.hidden = !full.hidden; more.setAttribute("aria-expanded", String(!full.hidden)); more.classList.toggle("open", !full.hidden); } });
    more.appendChild(chevron());
    (opts.quick || []).forEach(function (r) { row.append(btn(r)); }); if ((opts.all || []).length > (opts.quick || []).length) row.append(more);
    (opts.all || []).forEach(function (r) { full.append(btn(r)); });
    wrap.append(row, full); return wrap;
  }
  global.Emoji = { setStorage: setStorage, open: open, search: search, index: index, recent: recent, remember: remember, strip: strip, reactionBar: reactionBar };
})(typeof window !== "undefined" ? window : globalThis);
