// Barcode / QR scanning for equipment identifiers.
//
// Any button with data-scan="<css selector of input>" opens a camera dialog.
// The decoded text is written into that input; if data-scan-format is set,
// the barcode symbology (e.g. "ean_13") goes into that input.
//
// Engines, in order of preference:
//   1. The browser's native BarcodeDetector (Chrome/Edge on Android, ChromeOS, macOS).
//   2. ZXing (vendored at /static/vendor/zxing-library.min.js) for iOS Safari,
//      Firefox and others.
// The camera needs a secure context: HTTPS, or http://localhost.
(function () {
  "use strict";
  const FORMATS = ["qr_code", "ean_13", "ean_8", "upc_a", "upc_e", "code_128", "code_39", "code_93",
    "codabar", "itf", "data_matrix", "pdf417", "aztec"];
  const ZXING_URL = "/static/vendor/zxing-library.min.js";

  let zxingPromise = null;
  function loadZXing() {
    if (window.ZXing) return Promise.resolve(window.ZXing);
    if (!zxingPromise) {
      zxingPromise = new Promise((resolve, reject) => {
        const s = document.createElement("script");
        s.src = ZXING_URL;
        s.onload = () => (window.ZXing ? resolve(window.ZXing) : reject(new Error("ZXing failed to load")));
        s.onerror = () => reject(new Error("Could not load the barcode library"));
        document.head.append(s);
      });
    }
    return zxingPromise;
  }

  async function nativeDetector() {
    if (!("BarcodeDetector" in window)) return null;
    try {
      const supported = await window.BarcodeDetector.getSupportedFormats();
      const formats = FORMATS.filter((f) => supported.includes(f));
      return formats.length ? new window.BarcodeDetector({ formats }) : null;
    } catch (_) {
      return null;
    }
  }

  function zxingFormatName(ZXing, result) {
    const n = ZXing.BarcodeFormat[result.getBarcodeFormat()];
    return typeof n === "string" ? n.toLowerCase() : "";
  }

  function buildDialog() {
    const dlg = document.createElement("dialog");
    dlg.className = "scanner";
    dlg.innerHTML =
      '<div class="scanner-body">' +
      "<h2>Scan a barcode</h2>" +
      '<div class="scanner-view"><video playsinline muted></video></div>' +
      '<div class="scanner-status" aria-live="polite">Starting camera…</div>' +
      '<div class="actions">' +
      '<label class="btn"><input class="file-input" type="file" accept="image/*" capture="environment">Use a photo instead</label>' +
      '<button type="button" class="btn" data-close>Cancel</button>' +
      "</div></div>";
    document.body.append(dlg);
    return dlg;
  }

  async function scan(target, formatTarget) {
    const dlg = buildDialog();
    const video = dlg.querySelector("video");
    const status = dlg.querySelector(".scanner-status");
    const photo = dlg.querySelector('input[type="file"]');
    let stream = null, stopped = false, zxReader = null;

    function stop() {
      stopped = true;
      if (zxReader) try { zxReader.reset(); } catch (_) {}
      if (stream) stream.getTracks().forEach((t) => t.stop());
      if (dlg.open) dlg.close();
      dlg.remove();
    }
    function found(text, format) {
      if (stopped) return;
      target.value = text;
      target.dispatchEvent(new Event("input", { bubbles: true }));
      if (formatTarget) {
        formatTarget.value = format || "";
        const lbl = formatTarget.parentElement.querySelector("[data-format-label]");
        if (lbl) lbl.textContent = format ? "Scanned as " + format.replace(/_/g, "-").toUpperCase() : "";
      }
      if (navigator.vibrate) navigator.vibrate(80);
      stop();
    }

    dlg.querySelector("[data-close]").onclick = stop;
    dlg.addEventListener("cancel", stop);
    dlg.showModal();

    const detector = await nativeDetector();

    // Decode a still photo (fallback when live video is unavailable).
    photo.addEventListener("change", async () => {
      const file = photo.files[0];
      if (!file) return;
      status.textContent = "Reading photo…";
      try {
        if (detector) {
          const codes = await detector.detect(await createImageBitmap(file));
          if (codes.length) return found(codes[0].rawValue, codes[0].format);
        } else {
          const ZXing = await loadZXing();
          const url = URL.createObjectURL(file);
          try {
            const r = await new ZXing.BrowserMultiFormatReader().decodeFromImageUrl(url);
            return found(r.getText(), zxingFormatName(ZXing, r));
          } finally { URL.revokeObjectURL(url); }
        }
        status.textContent = "No barcode found in that photo. Try again closer and in focus.";
      } catch (_) {
        status.textContent = "No barcode found in that photo. Try again closer and in focus.";
      }
    });

    if (!window.isSecureContext || !navigator.mediaDevices || !navigator.mediaDevices.getUserMedia) {
      status.textContent = "Live camera needs HTTPS. You can still use a photo or type the code.";
      dlg.querySelector(".scanner-view").hidden = true;
      return;
    }

    try {
      if (detector) {
        stream = await navigator.mediaDevices.getUserMedia({
          video: { facingMode: { ideal: "environment" }, width: { ideal: 1280 }, height: { ideal: 720 } },
          audio: false,
        });
        video.srcObject = stream;
        await video.play();
        status.textContent = "Point the camera at the barcode.";
        const tick = async () => {
          if (stopped) return;
          try {
            const codes = await detector.detect(video);
            if (codes.length) return found(codes[0].rawValue, codes[0].format);
          } catch (_) {}
          setTimeout(tick, 150);
        };
        tick();
      } else {
        status.textContent = "Loading scanner…";
        const ZXing = await loadZXing();
        const hints = new Map();
        hints.set(ZXing.DecodeHintType.TRY_HARDER, true);
        zxReader = new ZXing.BrowserMultiFormatReader(hints);
        status.textContent = "Point the camera at the barcode.";
        await zxReader.decodeFromConstraints(
          { video: { facingMode: { ideal: "environment" } }, audio: false },
          video,
          (result) => { if (result) found(result.getText(), zxingFormatName(ZXing, result)); }
        );
      }
    } catch (err) {
      status.textContent = (err && err.name === "NotAllowedError")
        ? "Camera permission was denied. Allow camera access, use a photo, or type the code."
        : "Could not start the camera (" + ((err && err.message) || err) + "). Use a photo or type the code.";
    }
  }

  document.addEventListener("click", (e) => {
    const btn = e.target.closest("[data-scan]");
    if (!btn) return;
    e.preventDefault();
    const target = document.querySelector(btn.dataset.scan);
    const fmt = btn.dataset.scanFormat ? document.querySelector(btn.dataset.scanFormat) : null;
    if (target) scan(target, fmt);
  });

  // Typing a code by hand clears the remembered symbology.
  document.querySelectorAll("[data-scan-format]").forEach((btn) => {
    const target = document.querySelector(btn.dataset.scan);
    const fmt = document.querySelector(btn.dataset.scanFormat);
    if (target && fmt) target.addEventListener("keydown", () => { fmt.value = ""; });
  });
})();
