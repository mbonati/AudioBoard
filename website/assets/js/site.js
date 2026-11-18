/* AudioBoard website: small, dependency-free behaviours.
   Works from a static server and from file:// (no modules, no fetch). */
(function () {
  "use strict";

  var $ = function (s, r) { return (r || document).querySelector(s); };
  var $$ = function (s, r) { return Array.prototype.slice.call((r || document).querySelectorAll(s)); };
  var reduced = window.matchMedia && window.matchMedia("(prefers-reduced-motion: reduce)").matches;

  /* ---------------------------------------------------------------- nav */
  var nav = $(".nav");
  var toggle = $(".nav-toggle");
  if (nav && toggle) {
    toggle.addEventListener("click", function () {
      var open = nav.classList.toggle("open");
      toggle.setAttribute("aria-expanded", open ? "true" : "false");
    });
  }

  /* ------------------------------------------------------- reveal on scroll */
  var reveals = $$(".reveal");
  if ("IntersectionObserver" in window && !reduced) {
    var io = new IntersectionObserver(function (entries) {
      entries.forEach(function (e) {
        if (e.isIntersecting) { e.target.classList.add("in"); io.unobserve(e.target); }
      });
    }, { rootMargin: "0px 0px -60px 0px", threshold: 0.01 });
    reveals.forEach(function (el) { io.observe(el); });
  } else {
    reveals.forEach(function (el) { el.classList.add("in"); });
  }

  /* ----------------------------------------------------------- copy code */
  $$(".copy").forEach(function (b) {
    b.addEventListener("click", function () {
      // A one-line command says what to copy; a listing copies its text.
      var text = b.getAttribute("data-copy");
      if (text === null) {
        var block = b.closest(".code");
        var pre = block && block.querySelector("pre");
        text = pre ? pre.textContent : "";
      }
      var done = function () {
        b.textContent = "Copied"; b.classList.add("done");
        setTimeout(function () { b.textContent = "Copy"; b.classList.remove("done"); }, 1400);
      };
      if (navigator.clipboard && window.isSecureContext) {
        navigator.clipboard.writeText(text).then(done, function () { fallbackCopy(text); done(); });
      } else { fallbackCopy(text); done(); }
    });
  });
  function fallbackCopy(text) {
    var t = document.createElement("textarea");
    t.value = text; t.style.position = "fixed"; t.style.opacity = "0";
    document.body.appendChild(t); t.select();
    try { document.execCommand("copy"); } catch (e) { /* nothing to do */ }
    document.body.removeChild(t);
  }

  /* ------------------------------------------------------------ lightbox */
  // A modal <dialog>, so it stacks above the catalogue's inspector.
  var lb = document.createElement("dialog");
  lb.className = "lightbox";
  lb.innerHTML =
    '<div class="lb-bar"><span class="lb-title"></span><span class="grow"></span>' +
    '<button type="button" class="lb-size">Actual size</button>' +
    '<button type="button" class="lb-close">Close · Esc</button></div>' +
    '<div class="lb-inner"><img alt="" src="data:image/gif;base64,R0lGODlhAQABAAAAACw="></div>';
  document.body.appendChild(lb);
  var lbImg = $("img", lb), lbTitle = $(".lb-title", lb), lbSize = $(".lb-size", lb);
  var lastFocus = null;

  function openLightbox(img) {
    lastFocus = document.activeElement;
    lbImg.src = img.getAttribute("data-zoom") || img.currentSrc || img.src;
    lbImg.alt = img.alt || "";
    lbImg.classList.remove("full");
    lbSize.textContent = "Actual size";
    lbTitle.textContent = img.getAttribute("data-title") || img.alt || "";
    if (typeof lb.showModal === "function") lb.showModal(); else lb.setAttribute("open", "");
    document.body.style.overflow = "hidden";
    $(".lb-close", lb).focus();
  }
  function closeLightbox() {
    if (lb.open && typeof lb.close === "function") lb.close(); else lb.removeAttribute("open");
  }
  lb.addEventListener("close", function () {
    if (!document.querySelector("dialog.inspector[open]")) document.body.style.overflow = "";
    if (lastFocus && lastFocus.focus) lastFocus.focus();
  });
  lbSize.addEventListener("click", function () {
    var full = lbImg.classList.toggle("full");
    // Screenshots are rendered at 2x: "actual size" shows them pixel for pixel.
    lbImg.style.width = full ? (lbImg.naturalWidth / 2) + "px" : "";
    lbSize.textContent = full ? "Fit" : "Actual size";
  });
  $(".lb-close", lb).addEventListener("click", closeLightbox);
  lb.addEventListener("click", function (e) {
    if (e.target === lbImg || e.target.classList.contains("lb-inner")) closeLightbox();
  });
  function zoomable(img) {
    img.setAttribute("tabindex", "0");
    img.addEventListener("click", function (e) { e.stopPropagation(); openLightbox(img); });
    img.addEventListener("keydown", function (e) {
      if (e.key === "Enter" || e.key === " ") { e.preventDefault(); openLightbox(img); }
    });
  }
  $$("img[data-title], img[data-zoom]").forEach(function (img) {
    if (!img.closest(".card")) zoomable(img);
  });

  /* ----------------------------------------------- cables between steps */
  var steps = $(".steps[data-cables]");
  function drawCables() {
    if (!steps) return;
    var svg = $(".cables", steps);
    var box = steps.getBoundingClientRect();
    var pairs = [["#step1", "#step2"], ["#step2", "#step3"]];
    var colors = ["#60a5ee", "#e27096"];
    var html = "";
    pairs.forEach(function (p, i) {
      var a = $(p[0] + " .port.out", steps), b = $(p[1] + " .port.in", steps);
      if (!a || !b) return;
      var ra = a.getBoundingClientRect(), rb = b.getBoundingClientRect();
      var x1 = ra.left + ra.width / 2 - box.left, y1 = ra.top + ra.height / 2 - box.top;
      var x2 = rb.left + rb.width / 2 - box.left, y2 = rb.top + rb.height / 2 - box.top;
      var dx = Math.max(40, (x2 - x1) * 0.5);
      var d = "M" + x1 + " " + y1 + " C" + (x1 + dx) + " " + (y1 + 26) + " " + (x2 - dx) + " " + (y2 + 26) + " " + x2 + " " + y2;
      html += '<path d="' + d + '" stroke="' + colors[i] + '"/>';
      html += '<path class="flow" d="' + d + '" stroke="#e2e4e8"/>';
    });
    svg.innerHTML = html;
  }
  if (steps) {
    drawCables();
    window.addEventListener("resize", drawCables);
    window.addEventListener("load", drawCables);
    if (document.fonts && document.fonts.ready) document.fonts.ready.then(drawCables);
  }

  /* --------------------------------------------------------------- knobs */
  function polar(cx, cy, r, deg) {
    var a = (deg - 90) * Math.PI / 180;
    return [cx + r * Math.cos(a), cy + r * Math.sin(a)];
  }
  function arc(cx, cy, r, a0, a1) {
    var p0 = polar(cx, cy, r, a0), p1 = polar(cx, cy, r, a1);
    var large = a1 - a0 > 180 ? 1 : 0;
    return "M" + p0[0].toFixed(2) + " " + p0[1].toFixed(2) + " A" + r + " " + r + " 0 " + large + " 1 " + p1[0].toFixed(2) + " " + p1[1].toFixed(2);
  }
  var knobs = {};
  $$("[data-knob]").forEach(function (el) {
    var name = el.getAttribute("data-knob");
    var color = el.getAttribute("data-color") || "var(--blue)";
    var value = parseFloat(el.getAttribute("data-value") || "0.5");
    var initial = value;
    el.innerHTML =
      '<svg viewBox="0 0 46 46" aria-hidden="true">' +
      '<path class="trk" fill="none" stroke="#2e3036" stroke-width="3.5" stroke-linecap="round"/>' +
      '<path class="val" fill="none" stroke-width="3.5" stroke-linecap="round" style="stroke:' + color + '"/>' +
      '<circle cx="23" cy="23" r="12" fill="#24252a" stroke="#32343a"/>' +
      '<line class="ptr" x1="23" y1="23" stroke="#e2e4e8" stroke-width="2.2" stroke-linecap="round"/>' +
      "</svg>" +
      '<span class="label">' + el.getAttribute("data-label") + "</span>";
    el.setAttribute("role", "slider");
    el.setAttribute("tabindex", "0");
    el.setAttribute("aria-label", el.getAttribute("data-label"));
    el.setAttribute("aria-valuemin", "0");
    el.setAttribute("aria-valuemax", "100");
    var trk = $(".trk", el), val = $(".val", el), ptr = $(".ptr", el);
    trk.setAttribute("d", arc(23, 23, 19, -135, 135));
    function render() {
      var deg = -135 + value * 270;
      val.setAttribute("d", value > 0.004 ? arc(23, 23, 19, -135, deg) : "");
      var p = polar(23, 23, 9, deg);
      ptr.setAttribute("x2", p[0].toFixed(2));
      ptr.setAttribute("y2", p[1].toFixed(2));
      el.setAttribute("aria-valuenow", Math.round(value * 100));
    }
    function set(v) {
      value = Math.min(1, Math.max(0, v));
      render();
      if (knobs[name].onchange) knobs[name].onchange(value);
    }
    knobs[name] = { get: function () { return value; }, set: set, onchange: null };
    render();
    var startY = 0, startV = 0, dragging = false;
    el.addEventListener("pointerdown", function (e) {
      dragging = true; startY = e.clientY; startV = value;
      el.setPointerCapture(e.pointerId);
      e.preventDefault();
    });
    el.addEventListener("pointermove", function (e) {
      if (!dragging) return;
      var fine = e.shiftKey ? 0.25 : 1;
      set(startV + (startY - e.clientY) / 160 * fine);
    });
    var end = function () { dragging = false; };
    el.addEventListener("pointerup", end);
    el.addEventListener("pointercancel", end);
    el.addEventListener("dblclick", function () { set(initial); });
    el.addEventListener("wheel", function (e) { e.preventDefault(); set(value - e.deltaY / 1200); }, { passive: false });
    el.addEventListener("keydown", function (e) {
      var step = e.shiftKey ? 0.01 : 0.05;
      if (e.key === "ArrowUp" || e.key === "ArrowRight") { set(value + step); e.preventDefault(); }
      if (e.key === "ArrowDown" || e.key === "ArrowLeft") { set(value - step); e.preventDefault(); }
    });
  });

  /* ------------------------------------------- drone sketch and its scope */
  var canvas = $("#scope");
  var startBtn = $("#start");
  var synth = null;

  function makeSynth() {
    var AC = window.AudioContext || window.webkitAudioContext;
    if (!AC) return null;
    var ctx = new AC();
    var out = ctx.createGain(); out.gain.value = 0;
    var safety = ctx.createDynamicsCompressor();
    safety.threshold.value = -10; safety.ratio.value = 12; safety.attack.value = 0.003; safety.release.value = 0.15;
    var analyser = ctx.createAnalyser(); analyser.fftSize = 2048;
    var filter = ctx.createBiquadFilter(); filter.type = "lowpass"; filter.Q.value = 3.2;
    var dry = ctx.createGain(); dry.gain.value = 0.85;

    // A stereo tape echo: two delays fed back through a dark filter.
    var merger = ctx.createChannelMerger(2);
    var dl = ctx.createDelay(3), dr = ctx.createDelay(3);
    dl.delayTime.value = 0.62; dr.delayTime.value = 0.47;
    var tone = ctx.createBiquadFilter(); tone.type = "lowpass"; tone.frequency.value = 1900;
    var fb = ctx.createGain(); var wet = ctx.createGain();
    filter.connect(dl); filter.connect(dr);
    dl.connect(tone); dr.connect(tone);
    tone.connect(fb); fb.connect(dl); fb.connect(dr);
    dl.connect(merger, 0, 0); dr.connect(merger, 0, 1);
    merger.connect(wet); wet.connect(out);
    filter.connect(dry); dry.connect(out);
    out.connect(safety); safety.connect(analyser); analyser.connect(ctx.destination);

    // Eight detuned voices: a chord of fifths on A1.
    var notes = [55, 55, 82.41, 110, 110, 164.81, 220, 329.63];
    var drifts = [];
    notes.forEach(function (f, i) {
      var osc = ctx.createOscillator();
      osc.type = i % 3 === 2 ? "triangle" : "sawtooth";
      osc.frequency.value = f;
      osc.detune.value = (i % 2 ? 1 : -1) * (4 + i * 1.5);
      var lfo = ctx.createOscillator(); lfo.frequency.value = 0.04 + Math.random() * 0.13;
      var depth = ctx.createGain(); depth.gain.value = 14;
      lfo.connect(depth); depth.connect(osc.detune);
      var g = ctx.createGain(); g.gain.value = 0.11 / (1 + i * 0.22);
      var amp = ctx.createOscillator(); amp.frequency.value = 0.03 + Math.random() * 0.08;
      var ampDepth = ctx.createGain(); ampDepth.gain.value = g.gain.value * 0.45;
      amp.connect(ampDepth); ampDepth.connect(g.gain);
      var pan = ctx.createStereoPanner ? ctx.createStereoPanner() : null;
      osc.connect(g);
      if (pan) { pan.pan.value = ((i % 2 ? 1 : -1) * (0.25 + (i / notes.length) * 0.6)); g.connect(pan); pan.connect(filter); }
      else g.connect(filter);
      osc.start(); lfo.start(); amp.start();
      drifts.push(depth);
    });

    var s = { ctx: ctx, out: out, analyser: analyser, filter: filter, fb: fb, wet: wet, drifts: drifts, timer: null, base: 700 };
    s.apply = function () {
      var t = ctx.currentTime;
      s.base = 110 * Math.pow(2, knobs.tone.get() * 5.6);
      filter.frequency.setTargetAtTime(s.base, t, 0.25);
      drifts.forEach(function (d) { d.gain.setTargetAtTime(2 + knobs.drift.get() * 30, t, 0.3); });
      var sp = knobs.space.get();
      fb.gain.setTargetAtTime(0.18 + sp * 0.52, t, 0.3);
      wet.gain.setTargetAtTime(sp * 0.85, t, 0.3);
    };
    // The demo's Random: the cutoff wanders slowly around TONE.
    s.wander = function () {
      var target = s.base * Math.pow(2, Math.random() * 1.6 - 0.8);
      filter.frequency.setTargetAtTime(Math.min(9000, target), ctx.currentTime, 1.1);
    };
    return s;
  }

  function startSynth() {
    if (!synth) synth = makeSynth();
    if (!synth) return false;
    var s = synth, t;
    s.ctx.resume();
    s.apply();
    t = s.ctx.currentTime;
    s.out.gain.cancelScheduledValues(t);
    s.out.gain.setValueAtTime(s.out.gain.value, t);
    s.out.gain.linearRampToValueAtTime(0.5, t + 1.6);
    clearInterval(s.timer);
    s.timer = setInterval(s.wander, 2200);
    return true;
  }
  function stopSynth() {
    if (!synth) return;
    var s = synth, t = s.ctx.currentTime;
    clearInterval(s.timer);
    s.out.gain.cancelScheduledValues(t);
    s.out.gain.setValueAtTime(s.out.gain.value, t);
    s.out.gain.linearRampToValueAtTime(0, t + 0.9);
    setTimeout(function () { if (startBtn.getAttribute("aria-pressed") === "false") s.ctx.suspend(); }, 4000);
  }

  if (startBtn) {
    ["tone", "drift", "space"].forEach(function (k) {
      if (knobs[k]) knobs[k].onchange = function () { if (synth) synth.apply(); };
    });
    startBtn.addEventListener("click", function () {
      var on = startBtn.getAttribute("aria-pressed") !== "true";
      if (on && !startSynth()) return;
      if (!on) stopSynth();
      startBtn.setAttribute("aria-pressed", on ? "true" : "false");
      $(".txt", startBtn).textContent = on ? "Stop" : "Start";
    });
  }

  if (canvas) {
    var g2 = canvas.getContext("2d");
    var visible = true, buf = null;
    if ("IntersectionObserver" in window) {
      new IntersectionObserver(function (e) { visible = e[0].isIntersecting; if (visible) frame(); }).observe(canvas);
    }
    var running = false;
    function frame() {
      if (running) return;
      running = true;
      requestAnimationFrame(draw);
    }
    function draw(now) {
      running = false;
      var dpr = window.devicePixelRatio || 1;
      var w = canvas.clientWidth, h = canvas.clientHeight;
      if (canvas.width !== Math.round(w * dpr) || canvas.height !== Math.round(h * dpr)) {
        canvas.width = Math.round(w * dpr); canvas.height = Math.round(h * dpr);
      }
      g2.setTransform(dpr, 0, 0, dpr, 0, 0);
      g2.clearRect(0, 0, w, h);
      // grid
      g2.strokeStyle = "#1f2025"; g2.lineWidth = 1;
      for (var gx = 0; gx <= w; gx += w / 8) { g2.beginPath(); g2.moveTo(Math.round(gx) + 0.5, 0); g2.lineTo(Math.round(gx) + 0.5, h); g2.stroke(); }
      for (var gy = 0; gy <= h; gy += h / 4) { g2.beginPath(); g2.moveTo(0, Math.round(gy) + 0.5); g2.lineTo(w, Math.round(gy) + 0.5); g2.stroke(); }
      g2.strokeStyle = "#32343a"; g2.beginPath(); g2.moveTo(0, h / 2 + 0.5); g2.lineTo(w, h / 2 + 0.5); g2.stroke();

      var live = synth && startBtn.getAttribute("aria-pressed") === "true";
      var n = 240, i, x, y;
      g2.lineWidth = 2; g2.lineJoin = "round"; g2.lineCap = "round";
      g2.strokeStyle = live ? "#84c26c" : "#56bcac";
      g2.globalAlpha = live ? 1 : 0.75;
      g2.beginPath();
      if (live) {
        var a = synth.analyser;
        if (!buf || buf.length !== a.fftSize) buf = new Float32Array(a.fftSize);
        a.getFloatTimeDomainData(buf);
        // start on a rising zero crossing so the trace stands still
        var start = 0;
        for (i = 1; i < buf.length / 2; i++) { if (buf[i - 1] < 0 && buf[i] >= 0) { start = i; break; } }
        var span = Math.min(1024, buf.length - start);
        for (i = 0; i < span; i++) {
          x = i / (span - 1) * w;
          y = h / 2 - buf[start + i] * h * 0.9;
          if (i) g2.lineTo(x, y); else g2.moveTo(x, y);
        }
      } else {
        var t = (now || 0) / 1000;
        for (i = 0; i <= n; i++) {
          var u = i / n;
          var v = Math.sin(u * 18 + t * 1.3) * 0.5 + Math.sin(u * 27 - t * 0.7) * 0.25 + Math.sin(u * 9 + t * 0.4) * 0.35;
          v *= 0.32 + 0.08 * Math.sin(t * 0.6);
          x = u * w; y = h / 2 - v * h * 0.5;
          if (i) g2.lineTo(x, y); else g2.moveTo(x, y);
        }
      }
      g2.stroke();
      g2.globalAlpha = 1;
      g2.fillStyle = "#60646c";
      g2.font = "500 10px JetBrains Mono, ui-monospace, monospace";
      g2.fillText(live ? "OUT · LIVE" : "OUT · IDLE", 10, 16);
      if (visible && !reduced) frame();
    }
    frame();
    window.addEventListener("resize", frame);
  }

  /* ----------------------------------------------------------- catalogue */
  var catalogue = $("#catalogue-list");
  if (catalogue) {
    var cards = $$(".card", catalogue);
    var chips = $$(".chip[data-filter]");
    var search = $("#search");
    var empty = $(".empty");
    var current = "all";

    function apply() {
      var q = (search && search.value || "").trim().toLowerCase();
      var shown = 0;
      cards.forEach(function (c) {
        var okGroup = current === "all" || c.getAttribute("data-group") === current;
        var okText = !q || c.textContent.toLowerCase().indexOf(q) >= 0 || (c.getAttribute("data-keys") || "").indexOf(q) >= 0;
        var ok = okGroup && okText;
        c.style.display = ok ? "" : "none";
        if (ok) shown++;
      });
      $$(".cat-group", catalogue).forEach(function (g) {
        var any = $$(".card", g).some(function (c) { return c.style.display !== "none"; });
        g.style.display = any ? "" : "none";
      });
      if (empty) empty.style.display = shown ? "none" : "block";
    }
    chips.forEach(function (ch) {
      ch.addEventListener("click", function () {
        current = ch.getAttribute("data-filter");
        chips.forEach(function (o) { o.setAttribute("aria-pressed", o === ch ? "true" : "false"); });
        apply();
        if (current !== "all") {
          var sec = document.getElementById(current);
          if (sec) sec.scrollIntoView({ behavior: reduced ? "auto" : "smooth" });
        }
      });
    });
    if (search) search.addEventListener("input", apply);

    // Inspector
    var dlg = $("#inspector");
    function openCard(card) {
      if (!dlg) return;
      var accent = getComputedStyle(card).getPropertyValue("--accent");
      dlg.style.setProperty("--accent", accent);
      $(".in-title", dlg).textContent = card.getAttribute("data-name");
      $(".in-group", dlg).textContent = card.getAttribute("data-group-label");
      $(".in-after", dlg).textContent = card.getAttribute("data-after") || "";
      $(".in-after", dlg).style.display = card.getAttribute("data-after") ? "" : "none";
      var more = $(".more", card);
      $(".in-text-body", dlg).innerHTML = more ? more.innerHTML : "";
      var shot = $(".in-shot", dlg);
      var body = $(".in-body", dlg);
      var imgs = (card.getAttribute("data-shots") || "").split(",").filter(Boolean);
      shot.innerHTML = "";
      body.classList.toggle("noshot", !imgs.length);
      imgs.forEach(function (src, i) {
        var parts = src.split("|");
        var im = document.createElement("img");
        im.src = parts[0];
        im.alt = card.getAttribute("data-name") + " panel" + (parts[1] ? ", " + parts[1] : "");
        im.setAttribute("data-title", card.getAttribute("data-name") + (parts[1] ? " · " + parts[1] : ""));
        zoomable(im);
        if (i === 0) shot.appendChild(im);
        else {
          var extra = $(".extra", shot);
          if (!extra) {
            extra = document.createElement("div");
            // Many pictures (Iris's scenes): a grid of captioned thumbnails.
            extra.className = imgs.length > 4 ? "extra many" : "extra";
            shot.appendChild(extra);
          }
          if (imgs.length > 4) {
            var fig = document.createElement("figure");
            var cap = document.createElement("figcaption");
            cap.textContent = parts[1] || "";
            fig.appendChild(im);
            fig.appendChild(cap);
            extra.appendChild(fig);
          } else extra.appendChild(im);
        }
      });
      if (typeof dlg.showModal === "function") dlg.showModal(); else dlg.setAttribute("open", "");
      dlg.scrollTop = 0;
      if (history.replaceState) history.replaceState(null, "", "#" + card.id);
    }
    cards.forEach(function (c) {
      c.addEventListener("click", function () { openCard(c); });
      c.addEventListener("keydown", function (e) {
        if (e.key === "Enter" || e.key === " ") { e.preventDefault(); openCard(c); }
      });
    });
    if (dlg) {
      $(".in-close", dlg).addEventListener("click", function () { dlg.close(); });
      dlg.addEventListener("click", function (e) { if (e.target === dlg) dlg.close(); });
      dlg.addEventListener("close", function () {
        document.body.style.overflow = "";
        if (history.replaceState && location.hash.indexOf("#node-") === 0) history.replaceState(null, "", location.pathname + location.search);
      });
    }
    if (location.hash.indexOf("#node-") === 0) {
      var target = document.getElementById(location.hash.slice(1));
      if (target) { target.scrollIntoView(); openCard(target); }
    }
  }

  /* -------------------------------------------- table of contents (docs) */
  var toc = $(".toc");
  if (toc && "IntersectionObserver" in window) {
    var links = $$("a[href^='#']", toc);
    var map = {};
    links.forEach(function (a) { map[a.getAttribute("href").slice(1)] = a; });
    var seen = new IntersectionObserver(function (entries) {
      entries.forEach(function (e) {
        if (e.isIntersecting) {
          links.forEach(function (a) { a.classList.remove("active"); });
          var a = map[e.target.id];
          if (a) a.classList.add("active");
        }
      });
    }, { rootMargin: "-20% 0px -70% 0px" });
    Object.keys(map).forEach(function (id) { var s = document.getElementById(id); if (s) seen.observe(s); });
  }
})();
