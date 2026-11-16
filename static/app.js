// Greenhouse Maintenance Tracker — progressive enhancements.
// Everything here is optional: forms work without JavaScript.
(function () {
  "use strict";

  // ---------------------------------------------------------- timestamps
  // Server renders UTC; show the viewer's local time instead.
  const rel = new Intl.RelativeTimeFormat(undefined, { numeric: "auto" });
  function relative(d) {
    const s = (d.getTime() - Date.now()) / 1000;
    if (Math.abs(s) < 60) return rel.format(Math.round(s), "second");
    if (Math.abs(s) < 3600) return rel.format(Math.round(s / 60), "minute");
    if (Math.abs(s) < 86400) return rel.format(Math.round(s / 3600), "hour");
    if (Math.abs(s) < 604800) return rel.format(Math.round(s / 86400), "day");
    return null;
  }
  document.querySelectorAll("time.ts").forEach((t) => {
    const d = new Date(t.getAttribute("datetime"));
    if (isNaN(d)) return;
    const full = d.toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });
    t.title = full;
    t.textContent = (t.hasAttribute("data-relative") && relative(d)) || full;
  });

  // After posting a message, scroll to it (fetch() drops URL fragments,
  // so the server also passes ?posted=<comment id>).
  const posted = new URLSearchParams(location.search).get("posted");
  const anchor = location.hash ? decodeURIComponent(location.hash.slice(1)) : posted ? "c" + posted : "";
  const target = anchor && document.getElementById(anchor);
  if (target) {
    target.classList.add("highlight");
    target.scrollIntoView({ block: "center" });
  }

  // ---------------------------------------------------------- rich text editor
  // A contenteditable area with a small toolbar. The hidden <textarea> keeps
  // the HTML so the form posts normally; the server sanitises it.
  function escapeHtml(s) {
    return s.replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
  }
  function initEditor(root) {
    const area = root.querySelector(".editor-area");
    const toolbar = root.querySelector(".toolbar");
    const textarea = root.querySelector("textarea");
    const v = textarea.value.trim();
    area.innerHTML = v.includes("<") ? v : v ? escapeHtml(v).replace(/\n/g, "<br>") : "";
    area.hidden = false;
    toolbar.hidden = false;
    textarea.hidden = true;
    textarea.required = false;
    const sync = () => {
      // Treat an editor holding only whitespace / <br> as empty.
      textarea.value = area.textContent.trim() || area.querySelector("img") ? area.innerHTML : "";
    };
    area.addEventListener("input", () => {
      if (area.innerHTML === "<br>") area.innerHTML = "";
      sync();
    });
    toolbar.addEventListener("mousedown", (e) => e.preventDefault()); // keep selection
    toolbar.addEventListener("click", (e) => {
      const b = e.target.closest("button[data-cmd]");
      if (!b) return;
      area.focus();
      const cmd = b.dataset.cmd;
      if (cmd === "createLink") {
        const url = prompt("Link address (https://…)");
        if (url && /^(https?:|mailto:|tel:)/i.test(url.trim())) document.execCommand("createLink", false, url.trim());
      } else if (cmd === "formatBlock") {
        const inHeading = document.queryCommandValue("formatBlock").toLowerCase() === b.dataset.arg;
        document.execCommand("formatBlock", false, inHeading ? "p" : b.dataset.arg);
      } else {
        document.execCommand(cmd, false, null);
      }
      sync();
    });
    // Paste as plain text to avoid dragging in foreign styles.
    area.addEventListener("paste", (e) => {
      const text = (e.clipboardData || window.clipboardData).getData("text/plain");
      if (text) {
        e.preventDefault();
        document.execCommand("insertText", false, text);
      }
    });
    root._sync = sync;
    sync();
  }
  document.querySelectorAll("[data-editor]").forEach(initEditor);

  // ---------------------------------------------------------- photos
  // Collect photos from the camera / library inputs, preview them, shrink
  // them on the device (saves mobile data) and upload with fetch().
  const MAX_DIM = 1920;
  const QUALITY = 0.85;

  async function shrink(file) {
    if (!/^image\/(jpeg|png|webp|heic|heif)$/i.test(file.type) && !/\.(heic|heif)$/i.test(file.name)) return file;
    try {
      const bmp = await createImageBitmap(file, { imageOrientation: "from-image" });
      const scale = Math.min(1, MAX_DIM / Math.max(bmp.width, bmp.height));
      if (scale === 1 && file.size < 1.5 * 1024 * 1024 && /jpeg|png|webp/i.test(file.type)) return file;
      const canvas = document.createElement("canvas");
      canvas.width = Math.round(bmp.width * scale);
      canvas.height = Math.round(bmp.height * scale);
      canvas.getContext("2d").drawImage(bmp, 0, 0, canvas.width, canvas.height);
      const blob = await new Promise((r) => canvas.toBlob(r, "image/jpeg", QUALITY));
      if (!blob) return file;
      const name = file.name.replace(/\.[^.]+$/, "") + ".jpg";
      return new File([blob], name, { type: "image/jpeg" });
    } catch (_) {
      return file; // e.g. HEIC on a browser that cannot decode it
    }
  }

  function initPhotos(root) {
    const thumbs = root.querySelector(".thumbs");
    const files = [];
    root._files = files;
    function render() {
      thumbs.textContent = "";
      files.forEach((f, idx) => {
        const div = document.createElement("div");
        div.className = "thumb";
        const img = document.createElement("img");
        img.alt = f.name;
        img.src = URL.createObjectURL(f);
        img.onload = () => URL.revokeObjectURL(img.src);
        const rm = document.createElement("button");
        rm.type = "button";
        rm.textContent = "×";
        rm.setAttribute("aria-label", "Remove photo");
        rm.onclick = () => { files.splice(idx, 1); render(); };
        div.append(img, rm);
        thumbs.append(div);
      });
    }
    root.querySelectorAll('input[type="file"]').forEach((input) => {
      input.addEventListener("change", () => {
        const added = Array.from(input.files);
        files.push(...added);
        root.dispatchEvent(new CustomEvent("photos:added", { bubbles: true, detail: { files: added } }));
        input.value = ""; // allow taking another photo; files live in `files`
        render();
      });
    });
  }
  document.querySelectorAll("[data-photos]").forEach(initPhotos);

  document.querySelectorAll("form[data-upload]").forEach((form) => {
    form.addEventListener("submit", async (e) => {
      if (!window.fetch || !window.FormData) return; // plain submit
      e.preventDefault();
      form.querySelectorAll("[data-editor]").forEach((ed) => ed._sync && ed._sync());
      const btn = form.querySelector("[data-submit]");
      const label = btn ? btn.textContent : "";
      if (btn) { btn.disabled = true; btn.textContent = "Preparing photos…"; }
      try {
        const fd = new FormData(form);
        fd.delete("photos");
        const picker = form.querySelector("[data-photos]");
        for (const f of (picker && picker._files) || []) {
          const small = await shrink(f);
          fd.append("photos", small, small.name);
        }
        if (btn) btn.textContent = "Uploading…";
        const resp = await fetch(form.action, { method: "POST", body: fd, credentials: "same-origin" });
        if (resp.redirected) {
          window.location.href = resp.url;
          return;
        }
        const html = await resp.text();
        document.open();
        document.write(html);
        document.close();
      } catch (err) {
        alert("Upload failed — check your connection and try again.\n" + err);
        if (btn) { btn.disabled = false; btn.textContent = label; }
      }
    });
  });
})();
