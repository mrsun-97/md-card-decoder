// Engine loader + worker client (classic script; works on https:// and file://).
//
// https: the wasm is streamed and compiled from engine/decoder_bg.<hash>.wasm and workers load
//        engine/worker.<hash>.js.
// file:  fetch() and module workers are blocked, so the wasm comes from a classic script that sets
//        window.ENGINE_WASM_B64, and workers start from a Blob URL of window.ENGINE_WORKER_SOURCE.
// Card data always comes from a classic script (window.CARDS_BIN_B64), so both modes share one path.
(function () {
  'use strict';
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

  function fromBase64(b64) {
    if (Uint8Array.fromBase64) return Uint8Array.fromBase64(b64);
    const bin = atob(b64);
    const out = new Uint8Array(bin.length);
    for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
    return out;
  }

  let modulePromise = null;
  /** Compile the engine once per page; the Module is then shared with every worker. */
  function loadModule() {
    if (!modulePromise) {
      modulePromise = (async () => {
        if (isFile()) {
          await loadScript(files().wasmB64);
          return WebAssembly.compile(fromBase64(window.ENGINE_WASM_B64));
        }
        const res = await fetch(files().wasm);
        if (!res.ok) throw new Error(`HTTP ${res.status} for ${files().wasm}`);
        // Fall back if the server sends the wrong MIME type for .wasm.
        return (res.headers.get('content-type') || '').startsWith('application/wasm')
          ? WebAssembly.compileStreaming(res)
          : WebAssembly.compile(await res.arrayBuffer());
      })();
    }
    return modulePromise;
  }

  let cardsPromise = null;
  function loadCards() {
    if (!cardsPromise) {
      cardsPromise = (async () => {
        if (!window.CARDS_BIN_B64) await loadScript(files().cards);
        return fromBase64(window.CARDS_BIN_B64);
      })();
    }
    return cardsPromise;
  }

  async function workerUrl() {
    if (!isFile()) return files().worker;
    if (!window.ENGINE_WORKER_SOURCE) await loadScript(files().workerSource);
    return URL.createObjectURL(new Blob([window.ENGINE_WORKER_SOURCE], { type: 'text/javascript' }));
  }

  /** A worker running one engine instance, with promise-based requests. */
  class EngineWorker {
    static async create() {
      const [module, cards, url] = await Promise.all([loadModule(), loadCards(), workerUrl()]);
      const w = new EngineWorker(new Worker(url));
      const copy = cards.slice().buffer;
      await w.request({ type: 'init', module, cards: copy }, [copy]);
      return w;
    }
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
        const err = new Error(e.message || '引擎线程出错');
        for (const p of this.pending.values()) p.reject(err);
        this.pending.clear();
      };
    }
    request(msg, transfer = []) {
      const id = this.nextId++;
      return new Promise((resolve, reject) => {
        this.pending.set(id, { resolve, reject });
        this.worker.postMessage({ ...msg, id }, transfer);
      });
    }
    terminate() {
      this.worker.terminate();
      for (const p of this.pending.values()) p.reject(new Error('已取消'));
      this.pending.clear();
    }
  }

  window.EngineClient = { loadModule, loadCards, EngineWorker };
})();
