import { readFileSync, writeFileSync } from 'node:fs';
const names = ['play','pause','volume-2','volume-1','volume-x','pin','pin-off','maximize-2','minimize-2','x','minus','square','copy','settings','monitor','laptop','smartphone','sliders-horizontal','wifi','keyboard','info','audio-lines','check','chevron-down','search','cast','refresh-cw','arrow-left','circle-stop','mouse-pointer-2','gauge','plug','grip-vertical','loader-circle','list-video','speaker','eye','hard-drive','monitor-speaker','headphones','log-out','circle-x','expand','shrink','volume','sun','moon'];
const out = {};
for (const n of names) {
  const svg = readFileSync(`node_modules/lucide-static/icons/${n}.svg`, 'utf8');
  out[n] = svg.replace(/^[\s\S]*?<svg[^>]*>/, '').replace(/<\/svg>\s*$/, '').replace(/\n\s*/g, '');
}
writeFileSync('ui/icons.js', `// Generated from lucide-static by scripts-icons.mjs. Do not edit.\nconst ICONS=${JSON.stringify(out)};\nfunction icon(n,cls=''){return \`<svg class="ic \${cls}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">\${ICONS[n]||''}</svg>\`}\n`);
console.log('icons:', Object.keys(out).length);
