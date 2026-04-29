const { invoke } = window.__TAURI__.core;

// ── Constants ─────────────────────────────────────────────────────────────────

const START_HOUR = 6;
const END_HOUR = 23;
const RANGE_MIN = (END_HOUR - START_HOUR) * 60;
const SHORT_GAP_SECS = 300;

const DAY_NAMES = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'];
const MONTH_NAMES = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];

// ── Helpers ───────────────────────────────────────────────────────────────────

function pad2(n) { return String(n).padStart(2, '0'); }

function fmtTime(iso) {
  const d = new Date(iso);
  return `${pad2(d.getHours())}:${pad2(d.getMinutes())}`;
}

function fmtDur(startIso, endIso) {
  const secs = Math.round((new Date(endIso) - new Date(startIso)) / 1000);
  const h = Math.floor(secs / 3600);
  const m = Math.floor((secs % 3600) / 60);
  return `${h}h ${pad2(m)}m`;
}

function fmtTotal(secs) {
  if (secs === 0) return '—';
  const h = Math.floor(secs / 3600);
  const m = Math.floor((secs % 3600) / 60);
  return `${h}h ${pad2(m)}m`;
}

function topPct(iso) {
  const d = new Date(iso);
  const mins = d.getHours() * 60 + d.getMinutes() + d.getSeconds() / 60;
  return Math.max(0, Math.min(100, ((mins - START_HOUR * 60) / RANGE_MIN) * 100));
}

function mergeShortGaps(segments) {
  const merged = [];
  for (const seg of segments) {
    const prev = merged[merged.length - 1];
    if (prev && prev.end) {
      const gap = Math.round((new Date(seg.start) - new Date(prev.end)) / 1000);
      if (gap > 0 && gap < SHORT_GAP_SECS) {
        prev.end = seg.end;
        continue;
      }
    }
    merged.push({ ...seg });
  }
  return merged;
}

const MIN_BLOCK_SECS = 300;

function totalSecsForDay(segments) {
  return segments.reduce((acc, seg) => {
    if (!seg.end) return acc;
    const secs = Math.round((new Date(seg.end) - new Date(seg.start)) / 1000);
    return acc + secs;
  }, 0);
}

// ── Tooltip ───────────────────────────────────────────────────────────────────

const ttEl = document.getElementById('tooltip');
const ttTime = document.getElementById('tt-time');
const ttDur = document.getElementById('tt-dur');

function showTt(timeStr, durStr, x, y) {
  ttTime.textContent = timeStr;
  ttDur.textContent = durStr;
  ttEl.style.left = `${x + 14}px`;
  ttEl.style.top = `${Math.max(4, y - 52)}px`;
  ttEl.classList.remove('hidden');
}

function moveTt(x, y) {
  ttEl.style.left = `${x + 14}px`;
  ttEl.style.top = `${Math.max(4, y - 52)}px`;
}

function hideTt() { ttEl.classList.add('hidden'); }

// ── DOM Builders ──────────────────────────────────────────────────────────────

function buildSegment(seg, isToday) {
  const top = topPct(seg.start);
  const height = topPct(seg.end) - top;

  const el = document.createElement('div');
  el.className = [
    'absolute rounded-sm cursor-default transition-colors duration-100',
    isToday ? 'bg-blue-500 hover:bg-blue-400' : 'bg-sky-800 hover:bg-sky-700',
  ].join(' ');
  el.style.cssText = `top:${top}%;height:max(${height}%,6px);left:3px;right:3px;`;

  const actualSecs = Math.round((new Date(seg.end) - new Date(seg.start)) / 1000);
  const timeStr = `${fmtTime(seg.start)} – ${fmtTime(seg.end)}`;
  const durStr = fmtTotal(Math.max(actualSecs, MIN_BLOCK_SECS));
  el.addEventListener('mouseenter', (e) => showTt(timeStr, durStr, e.clientX, e.clientY));
  el.addEventListener('mousemove', (e) => moveTt(e.clientX, e.clientY));
  el.addEventListener('mouseleave', hideTt);
  return el;
}

function buildCalendar(history) {
  const todayStr = new Date().toISOString().slice(0, 10);
  const cal = document.getElementById('calendar');
  cal.innerHTML = '';

  // ── Time-label column ──
  const labelsCol = document.createElement('div');
  labelsCol.className = 'relative flex-shrink-0';
  labelsCol.style.width = '28px';

  const labelsArea = document.createElement('div');
  labelsArea.className = 'absolute inset-x-0 bottom-0';
  labelsArea.style.top = '60px';

  for (let h = START_HOUR; h <= END_HOUR; h++) {
    const pct = ((h - START_HOUR) / (END_HOUR - START_HOUR)) * 100;
    const lbl = document.createElement('div');
    lbl.className = 'absolute right-0 text-xs text-gray-600 leading-none tabular-nums';
    lbl.style.cssText = `top:${pct}%;transform:translateY(-50%);`;
    lbl.textContent = pad2(h);
    labelsArea.appendChild(lbl);
  }
  labelsCol.appendChild(labelsArea);
  cal.appendChild(labelsCol);

  // ── Day columns container ──
  const dayData = document.createElement('div');
  dayData.className = 'relative flex-1 min-w-0';

  const headersRow = document.createElement('div');
  headersRow.className = 'absolute inset-x-0 top-0 flex gap-2';
  headersRow.style.height = '60px';

  const gridsRow = document.createElement('div');
  gridsRow.className = 'absolute inset-x-0 bottom-0 flex gap-2';
  gridsRow.style.top = '60px';

  history.forEach((day) => {
    const isToday = day.date === todayStr;
    const date = new Date(day.date + 'T12:00:00');
    const segments = mergeShortGaps(day.segments);
    const secs = totalSecsForDay(segments);

    // Header cell
    const header = document.createElement('div');
    header.className = 'flex-1 flex flex-col items-center justify-center';
    header.innerHTML = `
      <span class="text-xs font-semibold ${isToday ? 'text-blue-400' : 'text-gray-500'}">${DAY_NAMES[date.getDay()]}</span>
      <span class="text-xs text-gray-600 mt-0.5">${MONTH_NAMES[date.getMonth()]} ${date.getDate()}</span>
      <span class="text-xs mt-0.5 font-medium ${secs > 0 ? (isToday ? 'text-blue-300' : 'text-sky-500') : 'text-gray-700'}">${fmtTotal(secs)}</span>
    `;
    headersRow.appendChild(header);

    // Grid cell
    const grid = document.createElement('div');
    grid.className = 'relative flex-1 rounded overflow-visible';
    grid.style.backgroundColor = isToday ? '#1e2a3a' : '#111827';

    // Hour guide lines
    for (let h = START_HOUR; h <= END_HOUR; h++) {
      const pct = ((h - START_HOUR) / (END_HOUR - START_HOUR)) * 100;
      const line = document.createElement('div');
      line.className = 'absolute inset-x-0 pointer-events-none';
      line.style.cssText = `top:${pct}%;border-top:1px solid #1f2937;`;
      grid.appendChild(line);
    }

    // Segment blocks
    segments.forEach((seg) => {
      if (!seg.end) return;
      grid.appendChild(buildSegment(seg, isToday));
    });

    gridsRow.appendChild(grid);
  });

  dayData.appendChild(headersRow);
  dayData.appendChild(gridsRow);
  cal.appendChild(dayData);
}

// ── Navigation ────────────────────────────────────────────────────────────────

let weekOffset = 0;

const rangeLabel = document.getElementById('range-label');
const btnPrev = document.getElementById('btn-prev');
const btnNext = document.getElementById('btn-next');
const btnCurrent = document.getElementById('btn-current');

function updateNavButtons() {
  btnNext.disabled = weekOffset === 0;
  btnCurrent.disabled = weekOffset === 0;
}

btnPrev.addEventListener('click', () => { weekOffset++; main(); });
btnNext.addEventListener('click', () => { if (weekOffset > 0) { weekOffset--; main(); } });
btnCurrent.addEventListener('click', () => { weekOffset = 0; main(); });

// ── Main ──────────────────────────────────────────────────────────────────────

async function main() {
  try {
    const history = await invoke('get_history', { weekOffset });
    if (weekOffset === 0) {
      rangeLabel.textContent = 'Last 7 Days';
    } else {
      const first = new Date(history[0].date + 'T12:00:00');
      const last = new Date(history[history.length - 1].date + 'T12:00:00');
      rangeLabel.textContent =
        `${MONTH_NAMES[first.getMonth()]} ${first.getDate()} – ${MONTH_NAMES[last.getMonth()]} ${last.getDate()}`;
    }
    updateNavButtons();
    buildCalendar(history);
  } catch (err) {
    document.getElementById('calendar').innerHTML =
      `<div class="text-red-400 text-sm self-center ml-4">Error: ${err}</div>`;
  }
}

main();
setInterval(main, 10_000);
