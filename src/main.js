import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

const QUALITIES = [
  {v:'auto',label:'Auto (best)'},{v:'4320p',label:'4320p · 8K'},{v:'2160p',label:'2160p · 4K'},
  {v:'1440p',label:'1440p · QHD'},{v:'1080p',label:'1080p · FHD'},{v:'720p',label:'720p · HD'},
  {v:'480p',label:'480p · SD'},{v:'360p',label:'360p'},{v:'240p',label:'240p'},{v:'144p',label:'144p'},
];
const CODECS = [{v:'auto',label:'Codec: Auto'},{v:'av1',label:'AV1'},{v:'vp9',label:'VP9'},{v:'avc',label:'AVC'}];

let route = { name: 'home' };
let settings = { download_dir:'', default_quality:'auto', default_codec:'auto', auto_sponsorblock:true, theme:'dark', hw_accel:true, check_updates:true };
let downloads = [];
let modal = null;
// Search results are expensive (a yt-dlp subprocess + thumbnail fetches), so
// cache them per query instead of re-running on every re-render.
const searchCache = new Map();
async function searchVideos(q){
  if(searchCache.has(q)) return searchCache.get(q);
  const p = api('search_videos',{query:q}).catch(err=>{
    // Never cache a failure — one transient error would poison the session
    // and every later visit would keep showing an empty grid.
    searchCache.delete(q);
    throw err;
  });
  searchCache.set(q,p);
  return p;
}

async function api(cmd, args={}){ try{ return await invoke(cmd, args); }catch(e){ console.error(cmd,e); throw e; } }
const $ = sel => document.querySelector(sel);
const esc = s => String(s??'').replace(/[&<>"']/g, c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
const fmtDur = s => s==null?'':`${Math.floor(s/3600)}:${String(Math.floor(s%3600/60)).padStart(2,'0')}:${String(s%60).padStart(2,'0')}`;

function shell(main){
  $('#app').innerHTML = `
  <header class="topbar">
    <div class="logo">Win<span>tube</span></div>
    <input id="q" placeholder="Search — press /"/> <button class="primary" id="go">Search</button>
    <span style="flex:1"></span>
    <button id="dl-btn">Downloads <span id="dl-n"></span></button>
    <button id="set-btn">⚙ Settings</button>
  </header>
  <div class="layout">
    <nav class="sidebar">
      <a href="#" data-r="home" class="${route.name==='home'?'active':''}">🏠 Home</a>
      <a href="#" data-r="shorts" class="${route.name==='shorts'?'active':''}">⚡ Shorts</a>
      <a href="#" data-r="history" class="${route.name==='history'?'active':''}">🕘 History</a>
      <a href="#" data-r="downloads" class="${route.name==='downloads'?'active':''}">⬇ Downloads</a>
      <a href="#" data-r="settings" class="${route.name==='settings'?'active':''}">⚙ Settings</a>
    </nav>
    <main class="main" id="content">${main}</main>
  </div>
  ${modal?`<div class="modal"><div class="sheet">${modal}</div></div>`:''}`;
  document.querySelectorAll('.sidebar a').forEach(a=>a.onclick=e=>{e.preventDefault();route={name:a.dataset.r};render();});
  $('#go').onclick=doSearch; $('#q').onkeydown=e=>{if(e.key==='Enter')doSearch();};
  document.addEventListener('keydown', e=>{ if(e.key==='/' && document.activeElement.tagName!=='INPUT'){e.preventDefault();$('#q')?.focus();} });
  $('#dl-btn').onclick=()=>{route={name:'downloads'};render();};
  $('#set-btn').onclick=()=>{route={name:'settings'};render();};
  bindCards();
  bindMain();
}

/// Wire up cards and inline action buttons that are injected after render().
function bindCards(){
  document.querySelectorAll('[data-watch]').forEach(el=>el.onclick=()=>{route={name:'watch',id:el.dataset.watch};render();});
  const retry = $('#retry');
  if(retry) retry.onclick = async ()=>{
    retry.disabled = true; retry.textContent = 'Retrying…';
    searchCache.clear();
    await render();
  };
  const diag = $('#diag');
  if(diag) diag.onclick = ()=>{ route={name:'settings'}; render(); };
}

async function doSearch(){ const q=$('#q').value.trim(); if(!q)return; route={name:'search',q}; await render(); }

const HOME_QUERY = 'trending music';
let renderToken = 0;
const skeletons = () => Array.from({length:8},()=>'<div class="card skel"><div class="ph"></div><div class="bar"></div><div class="bar short"></div></div>').join('');
const errorBox = msg => `<div class="errbox"><strong>Search failed</strong><p>${esc(msg)}</p><button class="primary" id="retry">Retry</button> <button id="diag">Check yt-dlp setup</button></div>`;

async function render(){
  const token = ++renderToken;
  if(route.name!=='downloads') stopDownloadsRefresh();
  try{
    if(route.name==='home' || route.name==='search'){
      // Paint the frame immediately and stream the grid in when the search
      // lands — the UI must never look frozen while yt-dlp runs.
      const q = route.name==='home' ? HOME_QUERY : route.q;
      const heading = route.name==='home' ? 'Home' : `Results for “${esc(route.q)}”`;
      shell(`<h2>${heading}</h2><div class="grid" id="grid">${skeletons()}</div>`);
      try{
        const vids = await searchVideos(q);
        if(token!==renderToken) return;
        const g = $('#grid');
        if(g) g.innerHTML = vids.length ? vids.map(card).join('') : '<p class="muted">No results found.</p>';
      }catch(e){
        if(token!==renderToken) return;
        const g = $('#grid');
        if(g) g.innerHTML = errorBox(e?.message||String(e));
      }
      bindCards();
      return;
    }
    if(route.name==='watch') return shell(await watchHTML(route.id));
    if(route.name==='history') return shell(await historyHTML());
    if(route.name==='downloads'){
      shell(await downloadsHTML());
      scheduleDownloadsRefresh();
      return;
    }
    if(route.name==='settings') return shell(await settingsHTML());
    return shell('<p>Shorts — pick a video from Home.</p>');
  }catch(e){ shell(`<p style="color:#f66">Error: ${esc(e.message||e)}</p>`); }
}
function card(v){ return `<div class="card" data-watch="${esc(v.id)}"><img src="${esc(v.thumbnail)}" loading="lazy" decoding="async"/><h3>${esc(v.title)}</h3><div class="muted">${esc(v.channel)}${v.duration?(' · '+fmtDur(v.duration)):''}</div></div>`; }

let currentInfo=null;
async function watchHTML(id){
  const info = await api('get_play_info',{videoId:id});
  currentInfo = info;
  const prog = info.best_progressive_url || info.formats.find(f=>f.is_progressive)?.url || info.dash_url || info.hls_url || '';
  const vid = info.video;
  const dur = fmtDur(vid.duration);
  const subs = (vid.subtitles||[]).slice(0,12).map(s=>`<option value="${esc(s.url)}">${esc(s.label)} (${esc(s.ext)})</option>`).join('');
  return `
  <div class="watch">
    <div>
      <video id="v" controls autoplay src="${esc(prog)}"></video>
      <h2 style="margin:12px 0 4px">${esc(vid.title)}</h2>
      <div class="muted">${esc(vid.channel)} · ${vid.view_count??''} views · ${esc(vid.upload_date||'')}${dur?' · '+dur:''}</div>
      <div class="row" style="margin-top:10px">
        <select id="q-sel">${QUALITIES.map(q=>`<option value="${q.v}" ${settings.default_quality===q.v?'selected':''}>${q.label}</option>`).join('')}</select>
        <select id="c-sel">${CODECS.map(c=>`<option value="${c.v}" ${settings.default_codec===c.v?'selected':''}>${c.label}</option>`).join('')}</select>
        <button class="primary" id="dl">⬇ Download</button>
        <button id="copy">🔗 Copy link</button>
      </div>
      <p style="white-space:pre-wrap;color:#ccc">${esc((vid.description||'').slice(0,1200))}</p>
    </div>
    <aside>
      <h3>Up next</h3>
      <div id="upnext" class="muted">Loading…</div>
      <h3 style="margin-top:16px">Subtitles</h3>
      <select id="sub" style="width:100%">${subs||'<option>No captions</option>'}</select>
      <h3 style="margin-top:16px">Formats (${info.formats.length})</h3>
      <div class="muted" style="max-height:300px;overflow:auto">${info.formats.slice(0,30).map(f=>`${esc(f.quality_label||f.format_id)} · ${esc(f.ext)} · ${f.height?f.height+'p':''}<br/>`).join('')}</div>
    </aside>
  </div>`;
}

async function historyHTML(){
  const h = await api('get_history',{limit:50}).catch(()=>[]);
  return `<h2>History</h2><div class="grid">${h.map(e=>`<div class="card" data-watch="${esc(e.video_id)}"><img src="${esc(e.thumbnail)}"/><h3>${esc(e.title)}</h3><div class="muted">${esc(e.channel)}</div></div>`).join('')||'<p class="muted">Nothing watched yet.</p>'}</div>`;
}

function updateDlBadge(){
  const n = downloads.filter(d=>d.status==='Pending' || d.status==='Downloading' || d.status==='Merging').length;
  const el = $('#dl-n');
  if(el) el.textContent = n ? `(${n})` : '';
}
function downloadsListHTML(){
  return `<h2>Downloads</h2>${downloads.map(d=>`
    <div style="border:1px solid #2a2a2a;border-radius:12px;padding:12px;margin:8px 0">
      <div class="row"><strong>${esc(d.title)}</strong><span class="muted">${esc(d.quality_label)} · ${esc(d.status)}</span><span style="flex:1"></span>
      ${d.status==='Downloading'?`<button data-cancel="${esc(d.id)}">Cancel</button>`:''}</div>
      <div class="progress" style="margin-top:8px"><i style="width:${Math.round((d.progress||0)*100)}%"></i></div>
      <div class="muted">${Math.round((d.progress||0)*100)}%${d.speed_bps?` · ${(d.speed_bps/1e6).toFixed(1)} MB/s`:''}${d.eta_secs?` · ETA ${d.eta_secs}s`:''}${d.error?` · <span style="color:#f66">${esc(d.error)}</span>`:''}</div>
    </div>`).join('')||'<p class="muted">No downloads yet. Open a video and press Download.</p>'}`;
}
function bindDownloads(){
  document.querySelectorAll('[data-cancel]').forEach(b=>b.onclick=async()=>{await api('cancel_download',{id:b.dataset.cancel}); render();});
}
// A download's status/progress only moves forward. When the list snapshot and
// an event race, keep whichever entry is further along instead of letting a
// late, older snapshot overwrite the page.
function dlRank(d){
  const order = {Pending:0, Downloading:1, Merging:2, Done:3, Error:3, Cancelled:3};
  return (order[d.status] ?? 0) * 2 + (d.progress || 0);
}
function mergeDownloads(fetched){
  const byId = new Map(fetched.map(d=>[d.id,d]));
  for(const d of downloads){
    const old = byId.get(d.id);
    if(!old || dlRank(d) >= dlRank(old)) byId.set(d.id,d);
  }
  downloads = [...byId.values()].sort((a,b)=>String(b.created_at||'').localeCompare(String(a.created_at||'')));
}
async function downloadsHTML(){
  const fetched = await api('list_downloads').catch(()=>null);
  if(fetched) mergeDownloads(fetched);
  updateDlBadge();
  return downloadsListHTML();
}
// Events are the fast path, but a dropped event must not freeze the page.
// Poll while something is in flight and repaint from the merged snapshot.
let downloadsTimer = null;
function stopDownloadsRefresh(){
  if(downloadsTimer){ clearTimeout(downloadsTimer); downloadsTimer = null; }
}
function scheduleDownloadsRefresh(){
  stopDownloadsRefresh();
  const tick = async ()=>{
    downloadsTimer = null;
    if(route.name!=='downloads') return;
    const fetched = await api('list_downloads').catch(()=>null);
    if(fetched){
      const before = JSON.stringify(downloads);
      mergeDownloads(fetched);
      updateDlBadge();
      if(JSON.stringify(downloads)!==before){
        const content = $('#content');
        if(content){ content.innerHTML = downloadsListHTML(); bindDownloads(); }
      }
    }
    if(downloads.some(d=>d.status==='Pending' || d.status==='Downloading' || d.status==='Merging')){
      downloadsTimer = setTimeout(tick, 1500);
    }
  };
  downloadsTimer = setTimeout(tick, 1500);
}

async function settingsHTML(){
  settings = await api('get_settings').catch(()=>settings);
  return `<h2>Settings</h2>
  <div class="sheet" style="max-width:560px">
    <label>Download folder<br/><input id="s-dir" value="${esc(settings.download_dir)}" style="width:100%"/></label><br/>
    <label>Default quality <select id="s-q">${QUALITIES.map(q=>`<option value="${q.v}" ${settings.default_quality===q.v?'selected':''}>${q.label}</option>`).join('')}</select></label>
    <label>Codec <select id="s-c">${CODECS.map(c=>`<option value="${c.v}" ${settings.default_codec===c.v?'selected':''}>${c.label}</option>`).join('')}</select></label><br/>
    <label><input type="checkbox" id="s-sb" ${settings.auto_sponsorblock?'checked':''}/> Auto-skip SponsorBlock</label><br/>
    <label><input type="checkbox" id="s-hw" ${settings.hw_accel?'checked':''}/> Hardware acceleration (mpv)</label><br/>
    <button class="primary" id="s-save">Save</button>
    <hr style="border-color:#333"/>
    <div>Sidecars: <code id="sidecars">checking…</code></div>
    <button id="s-check" style="margin:8px 0">🔍 Check yt-dlp setup</button>
    <div id="diag-out" class="muted" style="white-space:pre-wrap;line-height:1.6"></div>
    <p class="muted">yt-dlp ships inside the Wintube installer and must sit next to <code>Wintube.exe</code>. It resolves the 8K→144p quality ladder and drives the download remuxer. Every sidecar call is logged to <code id="logfile">…</code>.</p>
  </div>`;
}

function bindMain(){
  if($('#dl')) $('#dl').onclick = async ()=>{
    const q=$('#q-sel').value, c=$('#c-sel').value;
    try{
      await api('start_download',{videoId:route.id, title:currentInfo?.video?.title||route.id, quality:q, codec:c});
      modal = `<h3>Download started</h3><p class="muted">Quality ${q} · codec ${c}. Track progress in Downloads.</p><button class="primary" id="modal-ok">OK</button>`;
    }catch(e){
      modal = `<h3>Download failed</h3><p class="muted">${esc(e?.message||String(e))}</p><button class="primary" id="modal-ok">OK</button>`;
    }
    route.name='downloads'; render();
  };
  if($('#modal-ok')) $('#modal-ok').onclick = ()=>{ modal = null; render(); };
  if($('#copy')) $('#copy').onclick=()=>{navigator.clipboard?.writeText(`https://www.youtube.com/watch?v=${route.id}`);};
  if($('#sub')) $('#sub').onchange=e=>{ const v=$('#v'); if(v) v.textTracks?.[0] && (v.textTracks[0].mode='showing'); };
  document.querySelectorAll('[data-cancel]').forEach(b=>b.onclick=async()=>{await api('cancel_download',{id:b.dataset.cancel}); render();});
  if($('#s-save')) $('#s-save').onclick=async()=>{
    settings.download_dir=$('#s-dir').value; settings.default_quality=$('#s-q').value; settings.default_codec=$('#s-c').value;
    settings.auto_sponsorblock=$('#s-sb').checked; settings.hw_accel=$('#s-hw').checked;
    await api('save_settings',{settings}); alert('Saved');
  };
  if($('#sidecars')) api('ensure_sidecars').then(v=>{const el=$('#sidecars'); if(el) el.textContent=JSON.stringify(v);}).catch(()=>{});
  if($('#s-check')) $('#s-check').onclick=async()=>{
    const out=$('#diag-out'); if(out) out.textContent='Checking…';
    try{
      const d = await api('sidecar_diagnostics');
      const lines = [
        `yt-dlp path : ${d.yt_dlp}`,
        `found       : ${d.yt_dlp_found?'yes':'NO — reinstall Wintube'}`,
        `version     : ${d.yt_dlp_version||'—'}`,
        `ffmpeg      : ${d.ffmpeg_found?d.ffmpeg:'not found (only needed for AV1/VP9 downloads)'}`,
        `app folder  : ${d.exe_dir}`,
        `log file    : ${d.log_file}`,
        `probe search: ${d.probe_entries!=null?`ok (${d.probe_entries} result(s))`:(d.probe||'not run')}`,
      ];
      if(out) out.textContent=lines.join('\n');
      const lf=$('#logfile'); if(lf) lf.textContent=d.log_file;
    }catch(e){ if(out) out.textContent='Diagnostics failed: '+(e?.message||e); }
  };
  // save history + sponsorblock on watch
  if(route.name==='watch' && currentInfo){
    api('save_history',{entry:{video_id:route.id,title:currentInfo.video.title,channel:currentInfo.video.channel,thumbnail:currentInfo.video.thumbnail,position:0,duration:currentInfo.video.duration||0,watched_at:new Date().toISOString()}}).catch(()=>{});
    if(settings.auto_sponsorblock) api('get_sponsor_segments',{videoId:route.id}).then(segs=>{
      if(segs?.length){ const v=$('#v'); if(!v) return; const iv=setInterval(()=>{ if(!document.body.contains(v)){clearInterval(iv);return;} for(const s of segs){ const seg=s.segment||s; if(seg && v.currentTime>=seg[0] && v.currentTime<seg[1]){ v.currentTime=seg[1]; } } },500); }
    }).catch(()=>{});
  }
}

listen('download://progress', e=>{
  const job = e.payload;
  const index = downloads.findIndex(d=>d.id===job.id);
  if(index >= 0) downloads[index] = job; else downloads.unshift(job);
  updateDlBadge();
  // Repaint only the list from local state. A full render() would re-fetch
  // list_downloads for every tick, and out-of-order responses could leave the
  // page stuck at an old percentage after the job had already finished.
  if(route.name==='downloads'){
    const content = $('#content');
    if(content){ content.innerHTML = downloadsListHTML(); bindDownloads(); }
    if(downloads.some(d=>d.status==='Pending' || d.status==='Downloading' || d.status==='Merging')) scheduleDownloadsRefresh();
  }
}).catch(()=>{});

(async()=>{
  settings = await api('get_settings').catch(()=>settings);
  render();
})();
