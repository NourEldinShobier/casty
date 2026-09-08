const T = window.__TAURI__;
const { invoke } = T.core;
const win = T.window.getCurrentWindow();
const $ = (s) => document.querySelector(s);
const body = document.body;
const log = (m) => invoke('log', { msg: String(m) }).catch(() => {});
const fail = (m) => { const e = $('#err'); e.textContent = m; e.classList.add('on'); if (body.dataset.state === 'viewing') setStats(m); };
window.addEventListener('error', (e) => { fail('Error: ' + e.message); log('error ' + e.message + ' @' + e.lineno); });
window.addEventListener('unhandledrejection', (e) => { fail('Error: ' + (e.reason?.message || e.reason)); log('rejection ' + (e.reason?.message || e.reason)); });

const PRESETS = [
  { key: 'low', name: '640p', label: 'Low', width: 640, fps: 30, kbps: 1500 },
  { key: 'medium', name: '720p', label: 'Medium', width: 1280, fps: 30, kbps: 4000 },
  { key: 'high', name: '1080p', label: 'High', width: 1920, fps: 60, kbps: 10000 },
  { key: 'native', name: 'Native', label: 'Native', width: 0, fps: 60, kbps: 20000 },
];
const store = { get: (k, d) => { try { return JSON.parse(localStorage.getItem('casty.' + k)) ?? d; } catch { return d; } }, set: (k, v) => { try { localStorage.setItem('casty.' + k, JSON.stringify(v)); } catch {} } };
let preset = store.get('preset', 'medium');
let settings = { hide_controls_ms: 2000, keep_on_top: true };
let os = 'windows', myName = '';

// ---------- icons ----------
const setIcon = (sel, name) => document.querySelectorAll(sel).forEach((e) => (e.innerHTML = icon(name)));
setIcon('#settings1', 'settings'); $('#settings1').onclick = openSettings;
setIcon('#settings2,#settings3', 'settings');
setIcon('#wc-min,#wc-min2', 'minus'); setIcon('#wc-max', 'square'); setIcon('#wc-close,#wc-close2', 'x');
setIcon('#pause', 'pause'); setIcon('#hvolic', 'volume-2'); setIcon('#mute', 'volume-2');
setIcon('#pin', 'pin'); setIcon('#expand', 'expand'); setIcon('#back', 'log-out');
const ROUTES = [{ icon: 'monitor-speaker', tip: 'Sound plays on the PC' }, { icon: 'headphones', tip: 'Sound plays here, PC muted' }, { icon: 'audio-lines', tip: 'Sound plays on both' }];
function drawRoute() { const r = ROUTES[route]; $('#routebtn').innerHTML = icon(r.icon); $('#routebtn').dataset.tip = r.tip + ' \u00b7 click to change'; }
$('#stopcast').innerHTML = icon('circle-stop') + ' Stop';

// ---------- window chrome ----------
let maximized = false;
async function toggleMax() {
  await win.toggleMaximize();
  maximized = await win.isMaximized();
  body.classList.toggle('max', maximized);
  $('#wc-max').innerHTML = icon(maximized ? 'copy' : 'square');
  $('#expand').innerHTML = icon(maximized ? 'shrink' : 'expand'); $('#expand').dataset.tip = maximized ? 'Restore' : 'Expand';
}
document.querySelectorAll('[data-act]').forEach((b) => (b.onclick = (e) => {
  e.stopPropagation();
  const a = b.dataset.act;
  if (a === 'close') win.close();
  else if (a === 'min') win.minimize();
  else if (a === 'max') toggleMax();
  else if (a === 'settings') openSettings();
}));
document.querySelectorAll('.edge').forEach((e) => (e.onmousedown = (ev) => { if (ev.button === 0) win.startResizeDragging(e.dataset.d); }));
$('#grip').onmousedown = (ev) => { if (ev.button === 0) win.startResizeDragging('SouthEast'); };
$('#video').ondblclick = toggleMax;
$('#expand').onclick = toggleMax;
$('#pin').onclick = async () => {
  const on = $('#pin').getAttribute('aria-pressed') !== 'true';
  $('#pin').setAttribute('aria-pressed', on); $('#pin').innerHTML = icon(on ? 'pin' : 'pin-off'); $('#pin').dataset.tip = on ? 'Keep on top: on' : 'Keep on top: off';
  await win.setAlwaysOnTop(on);
};

// auto-hide controls
let hideT;
function showControls() {
  body.classList.add('controls');
  clearTimeout(hideT);
  if (!$('#qmenu').classList.contains('open')) hideT = setTimeout(() => body.classList.remove('controls'), settings.hide_controls_ms || 2000);
}
document.addEventListener('mousemove', showControls);
document.addEventListener('mouseleave', () => { clearTimeout(hideT); if (!$('#qmenu').classList.contains('open')) body.classList.remove('controls'); });
document.querySelector('.bar').addEventListener('mouseenter', () => clearTimeout(hideT));

async function openSettings() {
  const { WebviewWindow } = T.webviewWindow;
  const existing = await WebviewWindow.getByLabel('settings');
  if (existing) { await existing.show(); await existing.setFocus(); return; }
  const w = new WebviewWindow('settings', { url: 'settings.html', title: 'Casty Settings', width: 760, height: 540, minWidth: 560, minHeight: 400, decorations: false, transparent: true, shadow: false, center: true, alwaysOnTop: true });
  w.once('tauri://error', (e) => { log('settings window error ' + JSON.stringify(e.payload)); fail('Could not open settings: ' + JSON.stringify(e.payload)); });
  w.once('tauri://created', () => log('settings window created'));
  // note: never listen to tauri://close-requested here; a JS listener suppresses the default close
}

// ---------- window size per state ----------
const SIZES = { idle: [640, 400], viewing: [640, 400], hosting: [400, 310] };
let lastPlayerSize = store.get('size', SIZES.viewing);
async function setState(s) {
  const prev = body.dataset.state;
  if (prev === s) return;
  if (prev !== 'hosting') { try { const sz = await win.innerSize(); const f = await win.scaleFactor(); lastPlayerSize = [Math.round(sz.width / f), Math.round(sz.height / f)]; store.set('size', lastPlayerSize); } catch {} }
  body.dataset.state = s;
  if (maximized) await toggleMax();
  const [w, h] = s === 'hosting' ? SIZES.hosting : lastPlayerSize;
  try { await win.setSize(new T.dpi.LogicalSize(w, h)); } catch {}
}

// ---------- host side (this machine being watched) ----------
let hostInfo = null;
async function pollHost() {
  try { hostInfo = await invoke('host_info'); } catch { return; }
  os = hostInfo.os; body.dataset.os = os; myName = hostInfo.name;
  $('#me').textContent = `${hostInfo.name} · ${hostInfo.ips[0] || 'no network'}`;
  const hosting = hostInfo.viewers.length > 0;
  if (hosting && body.dataset.state !== 'viewing') await setState('hosting');
  else if (!hosting && body.dataset.state === 'hosting') await setState('idle');
  if (hosting) {
    $('#viewers').innerHTML = hostInfo.viewers.map((v) => `<span class="chip"><i>${(v.name[0] || '?').toUpperCase()}</i>${v.name} <span class="mono">${v.label}${v.audio ? ' · sound' : ''}</span></span>`).join('');
    const d = hostInfo.displays[0];
    $('#capsrc').textContent = d ? `${d.name || 'Display 1'}${d.width ? ` · ${d.width}×${d.height}` : ''}` : '';
    $('#hroute').querySelectorAll('button').forEach((b) => b.setAttribute('aria-pressed', +b.dataset.r === hostInfo.route));
    if (hostInfo.volume != null && document.activeElement !== $('#hvol')) setRange($('#hvol'), hostInfo.volume, $('#hvolv'));
  }
}
function setRange(el, v, label) { el.value = v; el.style.setProperty('--v', v + '%'); if (label) label.textContent = v; }
$('#hroute').querySelectorAll('button').forEach((b) => (b.onclick = () => { $('#hroute').querySelectorAll('button').forEach((x) => x.setAttribute('aria-pressed', x === b)); invoke('set_route', { route: +b.dataset.r }); }));
$('#hvol').oninput = () => { setRange($('#hvol'), $('#hvol').value, $('#hvolv')); invoke('set_volume', { pct: +$('#hvol').value }); };
$('#stopcast').onclick = async () => { const s = await invoke('get_settings'); await invoke('set_settings', { settings: { ...s, allow_viewers: false } }); setTimeout(() => invoke('set_settings', { settings: { ...s, allow_viewers: true } }), 3000); };
// ponytail: "Stop" closes viewers by refusing new sessions for 3 s; existing sockets end when their pipeline sees no consumer
invoke('role').then((r) => { if (r !== 'host') body.dataset.os = 'android'; });
setInterval(pollHost, 1000); pollHost().then(scan);

// ---------- discovery ----------
let devices = [];
async function scan() {
  $('#scan').innerHTML = icon('loader-circle') + ' Looking for devices…';
  try { devices = await invoke('discover'); } catch {}
  const n = drawList();
  $('#scan').innerHTML = icon('wifi') + ` ${n ? n + ' found' : 'Nothing found yet'} · <a href="#" id="rescan">search again</a>`;
  $('#rescan').onclick = (e) => { e.preventDefault(); scan(); };
}
function drawList() {
  $('#list').innerHTML = '';
  for (const d of devices.filter((d) => d.name !== myName && !(hostInfo?.ips || []).includes(d.ip))) {
    const b = document.createElement('button'); b.className = 'dev';
    b.innerHTML = `<span class="dic">${icon('monitor')}</span><span><b>${d.name}</b><span>Casty host</span></span><span class="mono">${d.ip}</span>`;
    b.onclick = () => connect(d.ip, d.port, d.name);
    $('#list').appendChild(b);
  }
  return $('#list').childElementCount;
}
$('#go').onclick = () => { const ip = $('#ip').value.trim(); if (ip) connect(ip, 45455, ip); };
$('#ip').onkeydown = (e) => { if (e.key === 'Enter') $('#go').click(); };

// ---------- viewer ----------
let ws, dec, raf, remote = null, display = null, route = 0, paused = false;
const canvas = $('#video'), ctx = canvas.getContext('2d', { alpha: false, desynchronized: true });
const stats = { frames: 0, bytes: 0, t: 0, lastFrame: null };
let actx, gain, nextT = 0;
function ensureAudio() {
  if (actx) return;
  actx = new AudioContext({ latencyHint: 'interactive' });
  gain = actx.createGain(); gain.gain.value = +$('#vol').value / 100; gain.connect(actx.destination);
}
function playAudio(buf) {
  ensureAudio();
  const dv = new DataView(buf); const rate = dv.getUint32(9, true), ch = dv.getUint8(13);
  const n = (buf.byteLength - 14) / 2 / ch; if (n <= 0) return;
  const ab = actx.createBuffer(ch, n, rate);
  const i16 = new Int16Array(buf.slice(14));
  for (let c = 0; c < ch; c++) { const d = ab.getChannelData(c); for (let i = 0; i < n; i++) d[i] = i16[i * ch + c] / 32768; }
  const src = actx.createBufferSource(); src.buffer = ab; src.connect(gain);
  const now = actx.currentTime;
  if (nextT < now + 0.02 || nextT > now + 0.5) nextT = now + 0.06; // resync after gaps or drift
  src.start(nextT); nextT += ab.duration;
}
function send(t) { if (ws?.open && ws.sid) fetch(`${ws.base}/ctl?sid=${ws.sid}`, { method: 'POST', body: t }).catch(() => {}); }

async function connect(ip, port, name) {
  stop(false);
  remote = { ip, port, name };
  $('#err').classList.remove('on');
  try { const r = await fetch(`http://${ip}:${port}/info`, { signal: AbortSignal.timeout(2500) }); remote.info = await r.json(); remote.name = remote.info.name; }
  catch { $('#err').textContent = `Cannot reach ${ip}. Is Casty running there and local network sharing on?`; $('#err').classList.add('on'); return; }
  const q = PRESETS.find((p) => p.key === preset);
  $('#title').textContent = remote.name; $('#qlabel').textContent = `${q.name} \u00b7 ${q.fps} fps`; $('#qname').textContent = q.name;
  await setState('viewing'); showControls();
  if (!('VideoDecoder' in window)) { setStats('WebCodecs not supported here'); return; }
  log('decoder setup, VideoDecoder=' + ('VideoDecoder' in window));
  dec = new VideoDecoder({ output: (f) => { if (stats.lastFrame) stats.lastFrame.close(); stats.lastFrame = f; stats.frames++; }, error: (e) => { log('decoder error ' + e.message); setStats('decoder: ' + e.message); } });
  dec.configure({ codec: 'avc1.42E02A', optimizeForLatency: true });
  let synced = false;
  const disp = display ?? remote.info.display ?? 0;
  const url = `http://${ip}:${port}/stream?width=${q.width}&fps=${q.fps}&kbps=${q.kbps}&display=${disp}&name=${encodeURIComponent(myName)}&audio=${route !== 0 ? 1 : 0}`;
  const ac = new AbortController();
  const onPacket = (buf) => {
    const dv = new DataView(buf), flags = dv.getUint8(0);
    stats.bytes += buf.byteLength;
    if (flags & 2) return playAudio(buf);
    const key = (flags & 1) === 1, ts = Number(dv.getBigUint64(1, true)) * 1000;
    if (!synced || dec.decodeQueueSize > 6) { if (!key) { synced = false; send('kf'); return; } synced = true; }
    if (dec.state !== 'configured') return;
    dec.decode(new EncodedVideoChunk({ type: key ? 'key' : 'delta', timestamp: ts, data: new Uint8Array(buf, 9) }));
  };
  ws = { abort: ac, sid: null, base: `http://${ip}:${port}`, open: false };
  const mine = ws;
  (async () => {
    let res;
    try { res = await fetch(url, { signal: ac.signal }); } catch (e) { if (ws === mine) { log('stream fetch failed ' + e.message); setStats(`cannot reach ${ip}:${port}`); } return; }
    if (!res.ok) { setStats(res.status === 403 ? 'the host is not accepting viewers' : `host error ${res.status}`); return; }
    mine.sid = res.headers.get('x-casty-session'); mine.open = true;
    setStats('waiting for keyframe\u2026'); send(`route:${route}`);
    const reader = res.body.getReader();
    let pend = new Uint8Array(0);
    try {
      for (;;) {
        const { value, done } = await reader.read();
        if (done || ws !== mine) break;
        if (pend.length) { const n = new Uint8Array(pend.length + value.length); n.set(pend); n.set(value, pend.length); pend = n; } else pend = value;
        let off = 0;
        while (pend.length - off >= 4) {
          const len = new DataView(pend.buffer, pend.byteOffset + off, 4).getUint32(0, true);
          if (pend.length - off - 4 < len) break;
          onPacket(pend.buffer.slice(pend.byteOffset + off + 4, pend.byteOffset + off + 4 + len));
          off += 4 + len;
        }
        pend = pend.subarray(off);
      }
    } catch (e) { if (ws === mine) log('stream read ended ' + e.message); }
    if (ws === mine) setStats('disconnected');
  })();
  const paint = () => {
    const f = stats.lastFrame;
    if (f) { if (canvas.width !== f.displayWidth || canvas.height !== f.displayHeight) { canvas.width = f.displayWidth; canvas.height = f.displayHeight; } ctx.drawImage(f, 0, 0); f.close(); stats.lastFrame = null; }
    raf = requestAnimationFrame(paint);
  };
  raf = requestAnimationFrame(paint);
  stats.frames = 0; stats.bytes = 0;
  stats.t = setInterval(() => {
    if (ws?.open && synced && !paused) $('#stats').innerHTML = `<span><b>${canvas.width}×${canvas.height}</b></span><span><b>${stats.frames}</b> fps</span><span><b>${(stats.bytes * 8 / 1e6).toFixed(1)}</b> Mb/s</span>`;
    stats.frames = 0; stats.bytes = 0;
  }, 1000);
  buildMenu();
}
function setStats(t) { $('#stats').textContent = t; }
function stop(toIdle = true) {
  clearInterval(stats.t); cancelAnimationFrame(raf);
  const w = ws; ws = null; w?.abort.abort();
  try { dec?.close(); } catch {} dec = null;
  stats.lastFrame?.close(); stats.lastFrame = null;
  nextT = 0; paused = false; $('#pause').setAttribute('aria-pressed', 'false'); $('#pause').innerHTML = icon('pause');
  $('#qmenu').classList.remove('open');
  if (toIdle) { remote = null; $('#title').textContent = 'Casty'; $('#qlabel').textContent = ''; setState('idle'); scan(); }
}
$('#back').onclick = () => stop(true);
$('#pause').onclick = () => {
  paused = !paused; $('#pause').setAttribute('aria-pressed', paused); $('#pause').dataset.tip = paused ? 'Resume' : 'Pause';
  $('#pause').innerHTML = icon(paused ? 'play' : 'pause'); send(`pause:${paused ? 1 : 0}`); if (paused) setStats('paused');
};
$('#routebtn').onclick = () => {
  route = (route + 1) % 3; drawRoute();
  send(`route:${route}`); send(`audio:${route !== 0 ? 1 : 0}`); if (route !== 0) ensureAudio();
  $('#volbox').dataset.tip = route === 0 ? "The PC's volume" : route === 1 ? 'Casty volume here' : "Casty here and the PC's volume";
};
drawRoute();
$('#mute').onclick = () => { const v = $('#vol'); v.value = v.value === '0' ? (v.dataset.last || 50) : (v.dataset.last = v.value, 0); v.oninput(); };
$('#vol').oninput = () => {
  const v = +$('#vol').value; setRange($('#vol'), v, $('#volv'));
  if (route !== 0 && gain) gain.gain.value = v / 100;
  if (route !== 1) send(`vol:${v}`);
  $('#mute').innerHTML = icon(v === 0 ? 'volume-x' : v < 50 ? 'volume-1' : 'volume-2');
};

function buildMenu() {
  const m = $('#qmenu'); const disps = remote?.info?.displays || [];
  const cur = display ?? remote?.info?.display ?? 0;
  m.innerHTML = '<div class="h">Quality</div>' + PRESETS.map((p) => `<button role="menuitemradio" aria-checked="${p.key === preset}" data-q="${p.key}">${p.label} <span class="mono">${p.name} · ${p.fps} · ${p.kbps / 1000} Mb/s</span></button>`).join('')
    + (disps.length > 1 ? '<div class="h">Display</div>' + disps.map((d, i) => `<button role="menuitemradio" aria-checked="${d.index === cur}" data-d="${d.index}">${d.name || 'Display ' + (i + 1)} <span class="mono">${d.width ? d.width + '×' + d.height : ''}</span></button>`).join('') : '');
  m.querySelectorAll('button').forEach((b) => (b.onclick = () => {
    if (b.dataset.q) { preset = b.dataset.q; store.set('preset', preset); } else display = +b.dataset.d;
    m.classList.remove('open'); if (remote) connect(remote.ip, remote.port, remote.name);
  }));
}
$('#qbtn').onclick = (e) => { e.stopPropagation(); $('#qmenu').classList.toggle('open'); showControls(); };
document.addEventListener('click', (e) => { if (!e.target.closest('#qmenu,#qbtn')) $('#qmenu').classList.remove('open'); });

// ---------- shortcuts ----------
document.addEventListener('keydown', (e) => {
  if (e.target.tagName === 'INPUT') return;
  if (e.key === 'Escape') { if (maximized) toggleMax(); else if (body.dataset.state === 'viewing') stop(true); }
  else if (e.key === ' ' && body.dataset.state === 'viewing') { e.preventDefault(); $('#pause').click(); }
  else if (e.key.toLowerCase() === 'f' && e.ctrlKey && e.shiftKey) toggleMax();
  else if (e.key.toLowerCase() === 'm' && body.dataset.state === 'viewing') { const v = $('#vol'); v.value = v.value === '0' ? 50 : 0; v.oninput(); }
});

// ---------- settings sync ----------
async function loadSettings() {
  try { settings = await invoke('get_settings'); } catch { return; }
  document.documentElement.dataset.theme = settings.theme === 'system' ? '' : settings.theme;
  await win.setAlwaysOnTop(!!settings.keep_on_top);
  $('#pin').setAttribute('aria-pressed', !!settings.keep_on_top); $('#pin').innerHTML = icon(settings.keep_on_top ? 'pin' : 'pin-off'); $('#pin').dataset.tip = settings.keep_on_top ? 'Keep on top: on' : 'Keep on top: off';
}
T.event.listen('settings', loadSettings);
loadSettings();
setRange($('#vol'), 50, $('#volv'));
