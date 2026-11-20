#!/usr/bin/env node
/** 只读：当前批次的工种回执是否齐全、独立、sha256 对得上实物、业务改动是否全部被认领。与 loop.mjs evidence 同一判据。 */
import { resolve } from 'node:path';
import * as L from './lib.mjs';
const { flags } = L.parseArgs(process.argv.slice(2));
const P = L.paths(resolve(typeof flags.repo === 'string' ? flags.repo : process.cwd()), flags);
const s = L.readState(P);
const b = L.currentBatch(s, typeof flags.batch === 'string' ? flags.batch : null);
if (!b) L.out({ verdict: 'FAIL', reason: 'NO_CURRENT_BATCH（先 init + dispatch）' }, 1);
const r = L.checkEvidence(P, s, b);
L.out({ verdict: r.reasons.length ? 'FAIL' : 'PASS', batch: b.id, required: L.requiredEvidence(P, b).map(x => x.file), rows: r.rows, notes: r.notes || [], reasons: r.reasons }, r.reasons.length ? 1 : 0);
