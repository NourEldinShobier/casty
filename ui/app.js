const { invoke } = window.__TAURI__.core;
const $ = (s) => document.querySelector(s);
const home = $('#home');

const PRESETS = {
  low:    { label: 'Low · 640p 30fps',     width: 640,  fps: 30, kbps: 1500 },
  medium: { label: 'Medium · 720p 30fps',  width: 1280, fps: 30, kbps: 4000 },
  high:   { label: 'High · 1080p 60fps',   width: 1920, fps: 60, kbps: 10000 },
  native: { label: 'Native · full res 60fps', width: 0, fps: 60, kbps: 20000 },
};
let preset = 'medium';
try { preset = localStorage.getItem('casty.preset') || preset; } catch {}

// ---------- host screen ----------
async function renderHost() {
  const info = await invoke('host_info');
  home.innerHTML = `
    <h1>Casty</h1><p class="sub">This ${navigator.platform.startsWith('Mac') ? 'Mac' : 'PC'} is ready to cast.</p>
    <div class="card">
      <div class="row"><span class="k">Name</span><span>${info.name}</span></div>
      <div class="row"><span class="k">Address</span><span class="mono">${info.ips.map(ip => `${ip}:${info.port}`).join('<br>') || 'no network'}</span></div>
      <div class="row"><span class="k">Viewers</span><span><span class="dot ${info.viewers ? 'on' : ''}"></span>${info.viewers}</span></div>
    </div>
    ${info.permission ? '' : `<div class="card"><p>Screen recording permission is required.</p><button class="primary" id="perm">Grant permission</button></div>`}
    <p class="sub">Open Casty on your phone and tap <b>Search for devices</b>. Manual entry works too.</p>
    <button class="link" id="toViewer">View another device from here →</button>`;
  $('#perm')?.addEventListener('click', () => invoke('request_permission').then(renderHost));
  $('#toViewer').onclick = () => renderViewer(true);
}

// ---------- viewer screen ----------
let devices = [];
function renderViewer(backToHost) {
  home.innerHTML = `
    <h1>Casty</h1><p class="sub">Watch a screen on this network.</p>
    <div class="card">
      <div class="row"><b>Devices</b><button id="scan">Search for devices</button></div>
      <div class="list" id="list"><p class="k" id="empty">Nothing found yet.</p></div>
    </div>
    <div class="card">
      <label>Quality</label>
      <select id="preset">${Object.entries(PRESETS).map(([k, p]) => `<option value="${k}" ${k === preset ? 'selected' : ''}>${p.label}</option>`).join('')}</select>
    </div>
    <div class="card">
      <label>Manual address</label>
      <div class="grid"><input id="ip" placeholder="192.168.1.20" inputmode="decimal"><button id="go">Connect</button></div>
    </div>
    ${backToHost ? '<button class="link" id="toHost">← Back to casting</button>' : ''}`;
  $('#preset').onchange = (e) => { preset = e.target.value; try { localStorage.setItem('casty.preset', preset); } catch {} };
  $('#scan').onclick = scan;
  $('#go').onclick = () => { const ip = $('#ip').value.trim(); if (ip) connect(ip, 45455); };
  $('#toHost')?.addEventListener('click', renderHost);
  drawList();
  scan();
}
async function scan() {
  const b = $('#scan'); b.disabled = true; b.textContent = 'Searching…';
  try { devices = await invoke('discover'); } catch (e) { console.error(e); }
  b.disabled = false; b.textContent = 'Search for devices';
  drawList();
}
function drawList() {
  const list = $('#list'); if (!list) return;
  list.innerHTML = devices.length ? '' : '<p class="k">Nothing found yet.</p>';
  for (const d of devices) {
    const b = document.createElement('button');
    b.innerHTML = `<span>${d.name}</span><span class="mono k">${d.ip}</span>`;
    b.onclick = () => connect(d.ip, d.port);
    list.appendChild(b);
  }
}

// ---------- player ----------
let ws, dec, raf;
const canvas = $('#canvas'), ctx = canvas.getContext('2d', { alpha: false, desynchronized: true });
const stats = { frames: 0, bytes: 0, t: 0, lastFrame: null };

function connect(ip, port) {
  stop();
  const q = PRESETS[preset];
  $('#player').classList.add('on');
  $('#stats').textContent = `connecting to ${ip}…`;
  if (!('VideoDecoder' in window)) { $('#stats').textContent = 'WebCodecs not supported on this device'; return; }

  dec = new VideoDecoder({
    output: (frame) => { if (stats.lastFrame) stats.lastFrame.close(); stats.lastFrame = frame; stats.frames++; },
    error: (e) => { $('#stats').textContent = 'decoder: ' + e.message; },
  });
  dec.configure({ codec: 'avc1.42E02A', optimizeForLatency: true });

  let synced = false;
  ws = new WebSocket(`ws://${ip}:${port}/stream?width=${q.width}&fps=${q.fps}&kbps=${q.kbps}`);
  ws.binaryType = 'arraybuffer';
  ws.onopen = () => { $('#stats').textContent = 'waiting for keyframe…'; };
  ws.onerror = () => { $('#stats').textContent = `cannot reach ${ip}:${port}`; };
  ws.onclose = () => { if (ws) $('#stats').textContent = 'disconnected'; };
  ws.onmessage = (e) => {
    const buf = e.data, dv = new DataView(buf);
    const key = (dv.getUint8(0) & 1) === 1;
    const ts = Number(dv.getBigUint64(1, true)) * 1000; // µs
    stats.bytes += buf.byteLength;
    // resync on a keyframe if we are not synced or the decoder is falling behind
    if (!synced || dec.decodeQueueSize > 6) {
      if (!key) { if (synced) synced = false; ws.send('kf'); return; }
      synced = true;
    }
    if (dec.state !== 'configured') return;
    dec.decode(new EncodedVideoChunk({ type: key ? 'key' : 'delta', timestamp: ts, data: new Uint8Array(buf, 9) }));
  };

  // paint the newest decoded frame once per display refresh; extra frames are skipped, never queued
  const paint = () => {
    const f = stats.lastFrame;
    if (f) {
      if (canvas.width !== f.displayWidth || canvas.height !== f.displayHeight) { canvas.width = f.displayWidth; canvas.height = f.displayHeight; }
      ctx.drawImage(f, 0, 0); f.close(); stats.lastFrame = null;
    }
    raf = requestAnimationFrame(paint);
  };
  raf = requestAnimationFrame(paint);
  stats.frames = 0; stats.bytes = 0;
  stats.t = setInterval(() => {
    if (ws?.readyState === 1 && synced) $('#stats').textContent = `${canvas.width}×${canvas.height} · ${stats.frames} fps · ${(stats.bytes * 8 / 1e6).toFixed(1)} Mbit/s`;
    stats.frames = 0; stats.bytes = 0;
  }, 1000);
}
function stop() {
  clearInterval(stats.t); cancelAnimationFrame(raf);
  const w = ws; ws = null; w?.close();
  try { dec?.close(); } catch {}
  dec = null;
  stats.lastFrame?.close(); stats.lastFrame = null;
  $('#player').classList.remove('on');
}
$('#stop').onclick = stop;

invoke('role').then((r) => (r === 'host' ? renderHost() : renderViewer(false)));
