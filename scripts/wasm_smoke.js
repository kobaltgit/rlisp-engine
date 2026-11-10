#!/usr/bin/env node
// node-смоук wasm-фасада: instantiate → загрузить сцену → кадры → пиксели/события
const fs = require('fs');
const path = require('path');

const WASM = process.argv[2] || 'target/wasm32-unknown-unknown/release/rlisp-engine.wasm';
const SCRIPT = process.argv[3] || 'assets/lisp/main_physpoly.lisp';
const FRAMES = parseInt(process.argv[4] || '30', 10);

(async () => {
  const bytes = fs.readFileSync(path.join(__dirname, '..', WASM));
  const mod = await WebAssembly.compile(bytes);
  const imports = WebAssembly.Module.imports(mod);
  console.log('imports:', JSON.stringify(imports.map(i => i.module + '.' + i.name)));
  const inst = await WebAssembly.instantiate(mod, {});
  const ex = inst.exports;
  console.log('exports:', Object.keys(ex).filter(k => k.startsWith('rl_')).join(' '), 'probe:', ex.rl_probe_embedded());

  const enc = new TextEncoder();
  const dec = new TextDecoder();
  const mem = () => new Uint8Array(ex.memory.buffer);
  const OUT = ex.rl_scratch_out();

  // корень загрузки: (load "x") резолвится от root — эмулируем cwd=assets/lisp
  process.chdir('assets/lisp');
  const src = fs.readFileSync(path.join(__dirname, '..', SCRIPT), 'utf8');
  const srcBytes = enc.encode(src);
  const IN = ex.rl_scratch_in();
  mem().set(srcBytes, IN);

  const S = ex.rl_new(320, 240);
  console.log('session ok, w=', ex.rl_w(S), 'h=', ex.rl_h(S));

  const readOut = (n) => dec.decode(new Uint8Array(ex.memory.buffer).subarray(OUT, OUT + n));

  let t0 = Date.now();
  if (ex.rl_load(S, IN, srcBytes.length) !== 0) {
    console.error('LOAD ERR:', readOut(ex.rl_last_err(0)));
    process.exit(1);
  }
  console.log('load ok in', Date.now() - t0, 'ms, байт:', srcBytes.length);

  t0 = Date.now();
  for (let i = 0; i < FRAMES; i++) {
    if (ex.rl_frame(S, 1 / 60, Date.now()) !== 0) {
      console.error('FRAME ERR:', readOut(ex.rl_last_err(0)));
      process.exit(1);
    }
  }
  const ms = Date.now() - t0;
  console.log(`${FRAMES} кадров за ${ms} мс (${(1000 / (ms / FRAMES)).toFixed(1)} fps wasm)`);

  const fb = new Uint8Array(ex.memory.buffer, ex.rl_fb(S), ex.rl_w(S) * ex.rl_h(S) * 4);
  let nz = 0, hash = 0;
  for (let i = 0; i < fb.length; i += 4) {
    if (fb[i] | fb[i + 1] | fb[i + 2]) nz++;
    hash = (Math.imul(hash, 131) + fb[i] + fb[i + 1] * 3 + fb[i + 2] * 7) | 0;
  }
  console.log('непустых пикселей:', nz, '/', ex.rl_w(S) * ex.rl_h(S), 'hash:', hash);

  const nEv = ex.rl_take_events(S, 4096);
  const evf = new Float32Array(new Uint8Array(ex.memory.buffer).buffer, OUT, nEv);
  console.log('событий физики:', nEv / 6, 'первые:', Array.from(evf.subarray(0, 12)).map(v => v.toFixed(2)).join(' '));

  const on = ex.rl_take_output(0);
  console.log('вывод движка:', JSON.stringify(readOut(on).slice(0, 200)));
})().catch(e => { console.error('FAIL:', e); process.exit(1); });
