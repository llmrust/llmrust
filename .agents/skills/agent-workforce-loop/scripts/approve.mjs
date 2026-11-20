#!/usr/bin/env node
/**
 * 用户本人的「同意」——只给用户在自己的终端里跑，agent 不要代跑。
 *
 *   node approve.mjs setup  --repo <仓库>   第一次：设审批密码，生成你的审批钥匙（私钥加密存你电脑主目录，公钥进仓库）
 *   node approve.mjs config --repo <仓库>   看过规则表后签名同意（绑定规则表当前内容）
 *   node approve.mjs spec   --repo <仓库>   acceptance.user_confirm=true 时，签名同意开工范围
 *   node approve.mjs whoami --repo <仓库>   看你的指纹，和仓库里登记的是否一致
 *
 * 必须在真实终端运行（要输入隐藏密码）；没有密码，任何人都签不出你的同意。
 * 改了规则表就要重新同意；开工后规则表再改，loop 一律拒绝（CONFIG_DRIFT）。
 */
import { readFileSync, writeFileSync, existsSync, mkdirSync } from 'node:fs';
import { join, dirname, resolve } from 'node:path';
import { homedir } from 'node:os';
import * as L from './lib.mjs';

const argv = process.argv.slice(2);
const cmd = argv[0];
const flag = (k) => { const i = argv.indexOf('--' + k); return i > -1 ? argv[i + 1] : undefined; };
const repo = resolve(flag('repo') || process.cwd());
const P = L.paths(repo, { dir: flag('dir') });
const KEY_HOME = process.env.AWL_APPROVER_HOME || join(homedir(), '.agent-workforce');
const KEY_FILE = join(KEY_HOME, 'approver-key.json');
const say = (s) => process.stdout.write(s + '\n');
const die = (s) => { say('\n✗ ' + s); process.exit(1); };

function ask(q, hidden = false) {
  return new Promise((res) => {
    process.stdout.write(q);
    const stdin = process.stdin; let buf = '';
    stdin.setRawMode(true); stdin.resume(); stdin.setEncoding('utf8');
    const on = (ch) => {
      for (const c of ch) {
        if (c === '\r' || c === '\n') { stdin.setRawMode(false); stdin.pause(); stdin.off('data', on); process.stdout.write('\n'); return res(buf); }
        if (c === '\u0003') { process.stdout.write('\n'); process.exit(130); }
        if (c === '\u007f' || c === '\b') { if (buf.length) { buf = buf.slice(0, -1); if (!hidden) process.stdout.write('\b \b'); } continue; }
        buf += c; if (!hidden) process.stdout.write(c);
      }
    };
    stdin.on('data', on);
  });
}

if (!['setup', 'config', 'spec', 'whoami'].includes(cmd)) die('用法：node approve.mjs setup|config|spec|whoami --repo <仓库>');
if (cmd !== 'whoami' && !(process.stdin.isTTY && process.stdout.isTTY)) die('APPROVE_NEEDS_TERMINAL：必须由你本人在自己的终端窗口里运行（要输入隐藏的审批密码）。agent 不能代跑。');

const local = existsSync(KEY_FILE) ? JSON.parse(readFileSync(KEY_FILE, 'utf8')) : null;
const repoPub = L.readApproverPub(P);

if (cmd === 'whoami') {
  say('你电脑上的审批指纹：' + (local ? local.fingerprint : '（还没设置，先跑 setup）'));
  say('仓库里登记的指纹：  ' + (repoPub.ok ? repoPub.fingerprint : '（未登记）'));
  if (local && repoPub.ok && local.fingerprint !== repoPub.fingerprint) die('两者不一致：仓库里登记的审批人不是你。不要签任何东西，先查是谁换的。');
  process.exit(0);
}

if (cmd === 'setup') {
  let store = local;
  if (!store) {
    say('第一次设置审批密码。这个密码只有你知道，不要告诉任何 AI，也不要写进任何文件。');
    const a = await ask('设置审批密码（至少 8 位，输入时不显示）：', true);
    const b = await ask('再输一遍：', true);
    if (a !== b) die('两次不一致');
    let made; try { made = L.makeApprover(a); } catch (e) { die(e.message); }
    mkdirSync(KEY_HOME, { recursive: true });
    writeFileSync(KEY_FILE, JSON.stringify(made.store, null, 2), { mode: 0o600 });
    store = made.store;
  }
  if (repoPub.ok && repoPub.fingerprint !== store.fingerprint) {
    say('⚠ 仓库里已经登记了另一个审批人（指纹 ' + repoPub.fingerprint + '），不是你的。');
    if ((await ask('确认要换成你自己，请输入「换成我」：')) !== '换成我') die('已取消');
  }
  mkdirSync(dirname(P.approverPub), { recursive: true });
  writeFileSync(P.approverPub, JSON.stringify({ schema: 'awl-approver-pub/v1', fingerprint: store.fingerprint, public_key_pem: store.public_key_pem, created_at: new Date().toISOString() }, null, 2) + '\n');
  say('\n✓ 审批人已登记。你的指纹：' + store.fingerprint);
  say('  记下这串指纹。以后机器报告里出现的审批指纹必须和它一样，不一样就是有人冒充。');
  process.exit(0);
}

// config / spec
if (!local) die('你还没设置审批密码，先跑：node approve.mjs setup --repo ' + repo);
if (!repoPub.ok) die('仓库里没有登记审批人，先跑：node approve.mjs setup --repo ' + repo);
if (repoPub.fingerprint !== local.fingerprint) die('仓库里登记的审批人（' + repoPub.fingerprint + '）不是你（' + local.fingerprint + '）。有人换了审批人，不要签。');
const target = cmd === 'config' ? P.config : P.spec;
if (!existsSync(target)) die('找不到 ' + L.relP(P, target));
const targetSha = L.shaFile(target);
say('\n要同意的文件：' + L.relP(P, target) + '（内容指纹 ' + targetSha.slice(0, 12) + '）');
if (cmd === 'config') {
  const c = JSON.parse(readFileSync(target, 'utf8'));
  say('要点（完整内容请在编辑器里看过再签）：');
  say('  业务代码目录：' + JSON.stringify(c.business_dirs));
  say('  每批必须跑过的检查：' + JSON.stringify(c.gates?.required_commands));
  say('  需要你开工前确认：' + (c.acceptance?.user_confirm ? '是' : '否（技术结果全由机器自证）'));
  say('  同时最多几批并行：' + (c.parallel?.max_batches ?? 2));
  say('  机器生成、不许手改的文件：' + JSON.stringify((c.generated || []).map(g => g.path)));
}
if ((await ask('\n确认你已看过并同意，请输入「同意」：')) !== '同意') die('未同意，什么都没记录');
const pw = await ask('输入审批密码：', true);
let priv; try { priv = L.unlockApprover(local, pw); } catch { die('密码不对'); }
const rec = L.signApproval(priv, local.fingerprint, cmd, L.relP(P, target), targetSha);
const f = L.approvalFile(P, cmd, targetSha);
mkdirSync(dirname(f), { recursive: true });
writeFileSync(f, JSON.stringify(rec, null, 2) + '\n');
say('\n✓ 已记录你的同意：' + L.relP(P, f));
say('  指纹 ' + local.fingerprint + '，时间 ' + rec.approved_at);
say('  这份文件以后改一个字，这条同意就失效，需要重新看、重新签。');
