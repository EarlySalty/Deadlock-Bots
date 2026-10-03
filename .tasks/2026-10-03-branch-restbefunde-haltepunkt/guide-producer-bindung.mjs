// Einmaliger Operationsstarter für den unveränderten freigegebenen Producer.
import fs from 'node:fs';
import path from 'node:path';
import { createHash } from 'node:crypto';
import { spawn, spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { validateTasks, readProcess, identity } from '/home/nathanael/Documents/.tasks/2026-10-02-offene-branches/status/sol-51eb094dc5242b90-common-checks/common-checks.mjs';

const base = '/home/nathanael/Documents/.tasks/2026-10-03-branch-restbefunde';
const guide = '/home/nathanael/Documents/.tasks/2026-10-03-serverguide/deploy-checks';
const producer = '/home/nathanael/Documents/.tasks/2026-10-02-offene-branches/status/sol-51eb094dc5242b90-common-checks';
const job = path.join(base, 'guide-producer-1d820bc2-7c2b0a08');
const unit = 'branch-restbefunde-guide-producer-1d820bc2-7c2b0a08-v2.service';
const sha = file => createHash('sha256').update(fs.readFileSync(file)).digest('hex');
const bindings = [
  [path.join(guide, 'producer-1d820bc2-7c2b0a08-check.json'), 'b45612a8a24893e4ef4e3cd32d38b4440dca89533ab9d5a458d9551f192799e5'],
  [path.join(guide, 'producer-1d820bc2-7c2b0a08-clippy.json'), 'cc963fb8257ae75a08c457ba8c4ecbddb4379f52f03fe22bbb29105cf5f78a36'],
  [path.join(producer, 'ENDMANIFEST.json'), '64ab1fc203cfd075abdbf4b0ba050120b8e3d8e6c15158de8985e0caa4cbae57'],
  [path.join(producer, 'common-checks.sh'), 'f7b0859b5b1478f063fc46a495d101d45a4629da1276f3562034ef3f104955e0'],
  [path.join(producer, 'common-checks.mjs'), '655d1ea5ac5408166d338d14056689a01b83ad6c09f0ff2c115606dce731cbf1'],
];
const freeze = path.join(guide, '1d820bc2-7c2b0a08-source-freeze');
const roots = { brain: '/home/nathanael/.worktrees/serverguide-deploy-20261003/Deadlock-Brain', bots: '/home/nathanael/.worktrees/serverguide-deploy-20261003/Deadlock-Bots', twitch: '/home/nathanael/.worktrees/serverguide-deploy-20261003/Deadlock-Twitch-Bot' };
const freezeBindings = fs.readdirSync(freeze).filter(name => name.endsWith('.sha256')).map(name => ({ file: path.join(freeze, name), sha256: sha(path.join(freeze, name)), repo: roots[name.split('-')[0]] }));
const manifests = bindings.slice(0, 2).map(([file]) => JSON.parse(fs.readFileSync(file, 'utf8')));

function verify() {
  for (const [file, expected] of bindings) if (sha(file) !== expected) throw new Error(`Dateibindung verändert: ${file}`);
  manifests.forEach(manifest => validateTasks(manifest.tasks));
  for (const item of freezeBindings) {
    if (!item.repo || sha(item.file) !== item.sha256) throw new Error('Sourcefreeze-Manifeste verändert.');
    const checked = spawnSync('/usr/bin/sha256sum', ['--check', '--status', item.file], { cwd: item.repo, stdio: 'ignore' });
    if (checked.status !== 0) throw new Error(`Sourcefreeze verletzt: ${path.basename(item.file)}`);
  }
  for (const task of manifests[0].tasks) {
    const git = args => spawnSync('/usr/bin/git', ['-C', task.repo, ...args], { encoding: 'utf8' });
    const head = git(['rev-parse', 'HEAD']), status = git(['status', '--porcelain', '--untracked-files=no']);
    if (head.status !== 0 || head.stdout.trim() !== task.sha || status.status !== 0 || status.stdout.trim()) throw new Error(`Quellkopf verändert: ${task.id}`);
  }
}

function duplicates() {
  const result = spawnSync('/usr/bin/rg', ['--files', '--hidden', '-g', 'input-manifest.json', '-g', '!target/**', '-g', '!node_modules/**', '/home/nathanael/Documents/.tasks/2026-10-03-serverguide', base, '/home/nathanael/Documents/.tasks/2026-10-02-offene-branches/status'], { encoding: 'utf8', maxBuffer: 8 * 1024 * 1024 });
  if (![0, 1].includes(result.status)) throw new Error('Doppelstartprüfung fehlgeschlagen.');
  return result.stdout.trim().split('\n').filter(Boolean).filter(file => {
    const j = JSON.parse(fs.readFileSync(file, 'utf8'));
    return j.tasks?.some(task => ['guide-brain-check', 'guide-bots-check', 'guide-brain-clippy', 'guide-bots-clippy'].includes(task.id));
  });
}

if (process.argv[2] === '--preflight') {
  verify();
  console.log(JSON.stringify({ bindings, freezeBindings, duplicates: duplicates(), launcher: readProcess(3488032), unit, compilerStarted: false }, null, 2));
} else if (process.argv[2] === '--sequence') {
  // Exklusives Anlegen verhindert auch zwei gleichzeitige eigene Starter.
  fs.mkdirSync(job);
  let active = null, interrupted = 0;
  const report = { startedAt: new Date().toISOString(), pid: process.pid, unit, bindings, freezeBindings, starterSha256: sha(fileURLToPath(import.meta.url)), phases: [], exitCode: null, ready_for_takeover: false, history: [{ unit: 'branch-restbefunde-guide-producer-1d820bc2-7c2b0a08.service', invocationID: '776d00f7de594d9dafa78e7cc5b02c1d', signal: 'ABRT', compilerStarted: false, producerStarted: false, finding: 'Importiertes Producermodul wertet --run selbst aus; eigenes Starterargument auf --sequence korrigiert.', log: path.join(base, 'guide-producer-bindung-unit.log') }] };
  const write = () => {
    const file = path.join(base, 'GUIDE-PRODUCER-BINDUNG.json');
    fs.writeFileSync(`${file}.tmp.${process.pid}`, `${JSON.stringify(report, null, 2)}\n`);
    fs.renameSync(`${file}.tmp.${process.pid}`, file);
    const stages = report.phases.map(item => `${item.name}: ${item.exitCode === undefined ? 'wartet oder läuft' : `Exit ${item.exitCode}`}`).join('\n\n');
    fs.writeFileSync(path.join(base, 'GUIDE-PRODUCER-BINDUNG.md'), `# Guide-Producerbindung\n\nEinmalige User-Unit: \`${unit}\`. Der freigegebene Producer bleibt unverändert. Check steht vor Clippy; Clippy startet ausschließlich nach zwei vollständig bestandenen Checkteilen und erneuter Quellenprüfung.\n\n${stages}\n\nTerminaler Starterexit: ${report.exitCode ?? 'offen'}. Übernahmebereitschaft: Nein. Serienprüfungen, Standardgates, minimale produktive Rechte und finale Ownerintegration bleiben offen.\n\nDie tatsächlichen Teilstarts, Quellen, Logs und Exits liegen im Laufverzeichnis \`${job}\`; die JSON-Datei enthält die aktuelle Bindung.\n`);
  };
  for (const [signal, code] of [['SIGTERM', 143], ['SIGINT', 130]]) process.on(signal, () => { interrupted = code; active?.kill(signal); });
  const guard = () => { if (interrupted) throw new Error('Eigener Starter wurde unterbrochen.'); };
  const wait = ms => new Promise(resolve => setTimeout(resolve, ms));
  const run = (manifest, dir, name) => new Promise((resolve, reject) => {
    const log = fs.openSync(path.join(job, `${name}-wrapper.log`), 'wx');
    active = spawn('/usr/bin/bash', [path.join(producer, 'common-checks.sh'), manifest, dir], { stdio: ['ignore', log, log] });
    fs.closeSync(log);
    const phase = { name, startedAt: new Date().toISOString(), wrapperPid: active.pid, runDir: dir };
    report.phases.push(phase); write();
    active.once('error', error => { active = null; reject(error); });
    active.once('exit', (code, signal) => { active = null; Object.assign(phase, { exitCode: code ?? (interrupted || 1), signal, finishedAt: new Date().toISOString() }); write(); resolve(phase); });
  });
  function completed(phase, manifest) {
    if (phase.exitCode !== 0) throw new Error(`${phase.name} hat Exit ${phase.exitCode}.`);
    const resultFile = path.join(phase.runDir, 'result.json');
    const result = JSON.parse(fs.readFileSync(resultFile, 'utf8'));
    if (fs.readFileSync(path.join(phase.runDir, 'exit-code'), 'utf8').trim() !== '0' || result.exitCode !== 0 || result.locksClosed !== true || result.parts?.length !== 2) throw new Error('Unvollständiger Producerabschluss.');
    for (const task of manifest.tasks) {
      const part = result.parts.find(item => item.id === task.id);
      if (!part || part.sha !== task.sha || part.exitCode !== 0 || part.validationExit !== 0 || !part.startedAt || !part.finishedAt || part.workerExit?.code !== 0) throw new Error(`Teilabschluss fehlt: ${task.id}`);
      const input = JSON.parse(fs.readFileSync(path.join(phase.runDir, task.id, 'source.json'), 'utf8'));
      if (input.sha !== task.sha || input.repo !== task.repo || JSON.stringify(input.args) !== JSON.stringify(task.args)) throw new Error('Teilquelle stimmt nicht mit dem Manifest überein.');
    }
    phase.resultSha256 = sha(resultFile); phase.parts = result.parts; write();
  }
  try {
    verify();
    const previous = duplicates();
    if (previous.length) throw new Error(`Vorhandene Paketbindung gefunden: ${previous.join(', ')}`);
    const launcher = readProcess(3488032);
    if (launcher && launcher.start === '118869336' && launcher.state !== 'Z') {
      report.phases.push({ name: 'Wartet auf vorher begonnenen Launcher-Cutover', launcher }); write();
      while (true) {
        guard(); const fresh = readProcess(launcher.pid);
        if (!fresh || (!fresh.uncertain && (identity(fresh) !== identity(launcher) || fresh.state === 'Z'))) break;
        await wait(1000);
      }
      Object.assign(report.phases.at(-1), { exitCode: 0, finishedAt: new Date().toISOString(), meaning: 'Vorheriger Launcherprozess beendet, kein Liveurteil übertragen' }); write();
    }
    guard(); verify();
    const check = await run(bindings[0][0], path.join(job, 'check'), 'check');
    guard(); completed(check, manifests[0]); verify();
    const clippy = await run(bindings[1][0], path.join(job, 'clippy'), 'clippy');
    guard(); completed(clippy, manifests[1]); verify();
    report.exitCode = 0;
  } catch (error) { report.exitCode = interrupted || 1; report.error = error.message; }
  report.finishedAt = new Date().toISOString(); write();
  fs.writeFileSync(path.join(job, 'exit-code'), `${report.exitCode}\n`);
  process.exitCode = report.exitCode;
} else throw new Error('Aufruf benötigt --preflight oder --sequence.');
