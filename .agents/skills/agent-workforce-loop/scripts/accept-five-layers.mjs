#!/usr/bin/env node
/** 只读：当前批次五层验收（事件→逻辑→接口→状态→视图）。与 loop.mjs accept 同一判据。 */
import { resolve } from 'node:path';
import * as L from './lib.mjs';
const { flags } = L.parseArgs(process.argv.slice(2));
const P = L.paths(resolve(typeof flags.repo === 'string' ? flags.repo : process.cwd()), flags);
const s = L.readState(P);
const b = L.currentBatch(s, typeof flags.batch === 'string' ? flags.batch : null);
if (!b) L.out({ verdict: 'FAIL', reason: 'NO_CURRENT_BATCH（先 init + dispatch）' }, 1);
const r = L.fiveLayers(P, s, b);
L.out({ verdict: r.reasons.length ? 'FAIL' : 'PASS', batch: b.id, layers: r.layers, overall: r.overall, reasons: r.reasons }, r.reasons.length ? 1 : 0);
