// Engine worker: one WASM instance built from a compiled WebAssembly.Module sent by the page.
// Bundled to a classic IIFE script so it also runs from a Blob URL (file:// pages).
import { initSync, Engine } from '../../build/wasm/decoder.js';

let engine = null;

self.onmessage = (event) => {
  const msg = event.data;
  try {
    let result;
    switch (msg.type) {
      case 'init':
        initSync({ module: msg.module });
        engine = new Engine(new Uint8Array(msg.cards), new Uint8Array(msg.tables || []));
        if (msg.policy) engine.setPolicy(msg.policy);
        result = { groups: engine.groups, dataHash: engine.dataHash };
        break;
      case 'policy':
        engine.setPolicy(msg.policy);
        result = true;
        break;
      case 'recommend':
        result = engine.recommend(msg.state);
        break;
      case 'simulate':
        result = engine.simulateBatch(msg.preset, msg.seedHi, msg.seedLo, msg.first, msg.count);
        break;
      case 'bench': {
        const t0 = performance.now();
        const cells = engine.benchPartitionAll(new Uint16Array(msg.cands));
        result = { cells, ms: performance.now() - t0 };
        break;
      }
      default:
        throw new Error(`unknown message ${msg.type}`);
    }
    self.postMessage({ id: msg.id, ok: true, result });
  } catch (err) {
    self.postMessage({ id: msg.id, ok: false, error: String((err && err.message) || err) });
  }
};
