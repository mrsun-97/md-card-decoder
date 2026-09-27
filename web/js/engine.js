// Engine access. The page owns one WASM instance on the main thread for cheap synchronous queries
// (candidates, reveal options, judging) and a Worker for recommendations; simulations use a pool
// of Workers. All instances come from one compiled WebAssembly.Module.
//
// https: wasm is streamed from engine/decoder_bg.<hash>.wasm and workers load engine/worker.<hash>.js.
// file:  fetch() and module workers are blocked, so the wasm comes from a classic script
//        (window.ENGINE_WASM_B64) and workers start from a Blob of window.ENGINE_WORKER_SOURCE.
import { initSync, Engine } from '../../build/wasm/decoder.js';

const files = () => window.ENGINE_FILES;
const isFile = () => location.protocol === 'file:';

function loadScript(src) {
  return new Promise((resolve, reject) => {
    const s = document.createElement('script');
    s.src = src;
    s.onload = resolve;
    s.onerror = () => reject(new Error(`无法加载 ${src}`));
    document.head.appendChild(s);
  });
}

export function fromBase64(b64) {
  if (Uint8Array.fromBase64) return Uint8Array.fromBase64(b64);
  const bin = atob(b64);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}

async function compileModule() {
  if (isFile()) {
    await loadScript(files().wasmB64);
    return WebAssembly.compile(fromBase64(window.ENGINE_WASM_B64));
  }
  const res = await fetch(files().wasm);
  if (!res.ok) throw new Error(`HTTP ${res.status}：${files().wasm}`);
  return (res.headers.get('content-type') || '').startsWith('application/wasm')
    ? WebAssembly.compileStreaming(res)
    : WebAssembly.compile(await res.arrayBuffer());
}

async function workerUrl() {
  if (!isFile()) return files().worker;
  if (!window.ENGINE_WORKER_SOURCE) await loadScript(files().workerSource);
  return URL.createObjectURL(new Blob([window.ENGINE_WORKER_SOURCE], { type: 'text/javascript' }));
}

/** A Worker running one engine instance, with promise-based requests. */
class EngineWorker {
  constructor(worker) {
    this.worker = worker;
    this.nextId = 1;
    this.pending = new Map();
    worker.onmessage = (e) => {
      const p = this.pending.get(e.data.id);
      if (!p) return;
      this.pending.delete(e.data.id);
      e.data.ok ? p.resolve(e.data.result) : p.reject(new Error(e.data.error));
    };
    worker.onerror = (e) => {
      e.preventDefault();
      this.failAll(new Error(e.message || '引擎线程出错'));
    };
  }
  request(msg, transfer = []) {
    const id = this.nextId++;
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.worker.postMessage({ ...msg, id }, transfer);
    });
  }
  failAll(err) {
    for (const p of this.pending.values()) p.reject(err);
    this.pending.clear();
  }
  terminate() {
    this.worker.terminate();
    this.failAll(new Error('已取消'));
  }
}

export class Engines {
  static async create(cards, tables) {
    const [module, url] = await Promise.all([compileModule(), workerUrl()]);
    initSync({ module });
    const e = new Engines();
    e.module = module;
    e.url = url;
    e.cards = cards;
    e.tables = tables;
    e.main = new Engine(cards, tables);
    e.policy = '{}';
    e.worker = await e.spawn();
    e.pool = [];
    return e;
  }

  async spawn() {
    const w = new EngineWorker(new Worker(this.url));
    await w.request({ type: 'init', module: this.module, cards: this.cards, tables: this.tables, policy: this.policy });
    return w;
  }

  setPolicy(policy) {
    this.policy = JSON.stringify(policy);
    this.main.setPolicy(this.policy);
    const all = [this.worker, ...this.pool].filter(Boolean);
    return Promise.all(all.map((w) => w.request({ type: 'policy', policy: this.policy })));
  }

  // Synchronous queries (main thread, a few milliseconds).
  columns() { return JSON.parse(this.main.columns()); }
  belief(state) { return JSON.parse(this.main.belief(JSON.stringify(state))); }
  revealOptions(state, field) { return JSON.parse(this.main.revealOptions(JSON.stringify(state), field)); }
  judge(guess, target) { return JSON.parse(this.main.judge(guess, target)); }

  /** Recommendation in the worker. A newer call cancels an older one (hard stop via terminate). */
  async recommend(state) {
    if (this.busy) {
      this.worker.terminate();
      this.worker = await this.spawn();
    }
    this.busy = true;
    try {
      return JSON.parse(await this.worker.request({ type: 'recommend', state: JSON.stringify(state) }));
    } finally {
      this.busy = false;
    }
  }

  /** Run `count` activities split over a pool of workers; `onProgress(done, merged)` per batch. */
  async simulate(preset, count, seed, onProgress, signal) {
    const size = Math.max(1, Math.min(8, (navigator.hardwareConcurrency || 4) - 1));
    while (this.pool.length < size) this.pool.push(await this.spawn());
    const merged = { solvedHist: new Array(preset.puzzles + 1).fill(0), hints: 0, challenges: 0, matches: 0, done: 0 };
    let next = 0;
    const batch = 4;
    const run = async (w) => {
      while (next < count && !signal?.aborted) {
        const first = next;
        const n = Math.min(batch, count - first);
        next += n;
        const r = JSON.parse(await w.request({ type: 'simulate', preset: JSON.stringify(preset), seedHi: 0, seedLo: seed >>> 0, first, count: n }));
        r.solvedHist.forEach((v, i) => { merged.solvedHist[i] += v; });
        merged.hints += r.hints;
        merged.challenges += r.challenges;
        merged.matches += r.matches;
        merged.done += n;
        onProgress?.(merged.done, merged);
      }
    };
    await Promise.all(this.pool.map(run));
    return merged;
  }

  stopSimulation() {
    for (const w of this.pool) w.terminate();
    this.pool = [];
  }
}
