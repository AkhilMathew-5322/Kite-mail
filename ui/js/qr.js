'use strict';
/* QR scanner shared by every platform: camera preview (getUserMedia) or a picture / pasted
   screenshot, decoded in Rust (qr_decode). Replaces the native Android scanner plugin. */

const QR_MAX_SIDE = 960; // px sent for decoding – enough for a code that fills a fifth of the frame

/** Bytes -> base64 without blowing the call stack on large frames. */
function bytesToB64(bytes) {
  let s = '';
  for (let i = 0; i < bytes.length; i += 0x8000) s += String.fromCharCode.apply(null, bytes.subarray(i, i + 0x8000));
  return btoa(s);
}

/** Looks for a QR code in a video frame, image or bitmap. Returns its text or null. */
async function qrFromSource(src, sw, sh, canvas) {
  if (!sw || !sh) return null;
  const k = Math.min(1, QR_MAX_SIDE / Math.max(sw, sh));
  const w = Math.max(1, Math.round(sw * k)), h = Math.max(1, Math.round(sh * k));
  canvas.width = w; canvas.height = h;
  const ctx = canvas.getContext('2d', { willReadFrequently: true });
  ctx.drawImage(src, 0, 0, w, h);
  const rgba = ctx.getImageData(0, 0, w, h).data;
  const gray = new Uint8Array(w * h);
  for (let i = 0, j = 0; j < gray.length; i += 4, j++) gray[j] = (rgba[i] * 77 + rgba[i + 1] * 150 + rgba[i + 2] * 29) >> 8;
  return invoke('qr_decode', { pixels: bytesToB64(gray), width: w });
}

/** Text of the first QR code in an image file / clipboard image, or null. */
async function qrFromFile(file, canvas) {
  const bmp = await createImageBitmap(file);
  try {
    // small codes in big screenshots need full size; try that after the quick downscaled pass
    return (await qrFromSource(bmp, bmp.width, bmp.height, canvas))
      || (Math.max(bmp.width, bmp.height) > QR_MAX_SIDE ? await qrFullSize(bmp, canvas) : null);
  } finally { bmp.close?.(); }
}
async function qrFullSize(bmp, canvas) {
  const side = Math.min(2400, Math.max(bmp.width, bmp.height));
  const k = side / Math.max(bmp.width, bmp.height);
  const w = Math.round(bmp.width * k), h = Math.round(bmp.height * k);
  canvas.width = w; canvas.height = h;
  const ctx = canvas.getContext('2d', { willReadFrequently: true });
  ctx.drawImage(bmp, 0, 0, w, h);
  const rgba = ctx.getImageData(0, 0, w, h).data;
  const gray = new Uint8Array(w * h);
  for (let i = 0, j = 0; j < gray.length; i += 4, j++) gray[j] = (rgba[i] * 77 + rgba[i + 1] * 150 + rgba[i + 2] * 29) >> 8;
  return invoke('qr_decode', { pixels: bytesToB64(gray), width: w });
}

/**
 * Opens the scanner. `check(text)` returns '' to accept a code or a message to keep scanning.
 * Resolves with the accepted text, or null if the user closed the scanner.
 */
function scanQr({ title = 'Scan QR code', hint = '', check = () => '' } = {}) {
  return new Promise((resolve) => {
    const d = document.createElement('dialog');
    d.className = 'sheet narrow qr-dlg';
    d.innerHTML = `<form method="dialog" onsubmit="return false"><header><strong>${esc(title)}</strong><span class="grow"></span><button type="button" class="icon" data-x>${icon('x', 18)}</button></header>
      ${hint ? `<p class="muted small">${hint}</p>` : ''}
      <div class="qr-view"><video playsinline muted autoplay></video><div class="qr-aim"></div></div>
      <p class="small qr-msg" role="status">Starting the camera…</p>
      <div class="set-actions">
        <button type="button" class="ghost" data-pick>${icon('image', 15)} Choose picture</button><input type="file" accept="image/*" hidden>
        <button type="button" class="ghost" data-flip hidden>${icon('refresh', 15)} Switch camera</button>
      </div>
      <p class="muted small">${S.mobile ? 'Or pick a screenshot of the code.' : 'Or pick a screenshot / photo of the code, or paste one here (Ctrl+V).'}</p></form>`;
    document.body.appendChild(d);
    const video = $('video', d), msg = $('.qr-msg', d), canvas = document.createElement('canvas');
    let stream = null, timer = null, busy = false, done = false, facing = 'environment', lastBad = '';

    const stop = () => {
      clearInterval(timer); timer = null;
      stream?.getTracks().forEach((t) => t.stop()); stream = null;
      video.srcObject = null;
    };
    const finish = (text) => {
      if (done) return;
      done = true;
      stop();
      if (d.open) d.close();
      d.remove();
      dropLayer('qr');
      resolve(text);
    };
    const found = (text) => {
      if (!text || done) return false;
      const problem = check(text);
      if (!problem) { finish(text); return true; }
      if (text !== lastBad) { lastBad = text; msg.textContent = problem; msg.className = 'small qr-msg bad'; }
      return false;
    };
    const tick = async () => {
      if (busy || done || video.readyState < 2) return;
      busy = true;
      try { found(await qrFromSource(video, video.videoWidth, video.videoHeight, canvas)); } catch (_) { /* next frame */ }
      busy = false;
    };
    const start = async () => {
      stop();
      if (!navigator.mediaDevices?.getUserMedia) { msg.textContent = 'No camera access here – choose a picture of the code instead.'; return; }
      try {
        stream = await navigator.mediaDevices.getUserMedia({ audio: false, video: { facingMode: { ideal: facing }, width: { ideal: 1280 }, height: { ideal: 720 } } });
        if (done) { stop(); return; }
        video.srcObject = stream;
        video.classList.toggle('mirror', facing === 'user');
        await video.play().catch(() => {});
        msg.textContent = 'Point the camera at the QR code';
        msg.className = 'small qr-msg';
        timer = setInterval(tick, 250);
        const cams = (await navigator.mediaDevices.enumerateDevices().catch(() => [])).filter((x) => x.kind === 'videoinput');
        $('[data-flip]', d).hidden = cams.length < 2;
      } catch (e) {
        msg.className = 'small qr-msg bad';
        msg.textContent = e?.name === 'NotAllowedError' ? 'Camera access was refused. Allow it in the system settings, or choose a picture of the code.'
          : e?.name === 'NotFoundError' || e?.name === 'OverconstrainedError' ? 'No camera found – choose a picture of the code instead.'
          : `Camera unavailable (${e?.message || e?.name || e}) – choose a picture of the code instead.`;
        $('.qr-view', d).classList.add('off');
      }
    };
    const fromFile = async (file) => {
      if (!file) return;
      msg.className = 'small qr-msg';
      msg.textContent = 'Reading the picture…';
      try {
        const text = await qrFromFile(file, canvas);
        if (!text) { msg.className = 'small qr-msg bad'; msg.textContent = 'No QR code found in that picture. Try a sharper or closer one.'; return; }
        lastBad = '';
        found(text);
      } catch (e) { msg.className = 'small qr-msg bad'; msg.textContent = `Couldn't read that picture: ${errText(e)}`; }
    };

    $('[data-x]', d).onclick = () => finish(null);
    d.addEventListener('cancel', (e) => { e.preventDefault(); finish(null); });
    $('[data-pick]', d).onclick = () => $('input[type=file]', d).click();
    $('input[type=file]', d).onchange = (e) => fromFile(e.target.files[0]);
    $('[data-flip]', d).onclick = () => { facing = facing === 'environment' ? 'user' : 'environment'; start(); };
    d.addEventListener('paste', (e) => {
      const item = [...(e.clipboardData?.items || [])].find((i) => i.type.startsWith('image/'));
      if (item) { e.preventDefault(); fromFile(item.getAsFile()); }
    });
    d.showModal();
    document.activeElement?.blur(); // no focus ring on the close button when it opens
    pushLayer('qr', () => finish(null));
    start();
  });
}

/** Pairing codes made by Kite Mail on another computer or phone. */
const isPairCode = (t) => /^kitemail:\/\/pair\?/i.test(String(t).trim());
const pairCheck = (t) => (isPairCode(t) ? '' : "That QR code isn't a Kite Mail pairing code. On the other device open Settings → Pair a device → Show pairing QR code.");
