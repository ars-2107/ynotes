// Exercise the packed launcher and native package without publishing or
// changing the user's global installation. Keep the temporary artefacts for
// inspection, including on failure. Run on a POSIX host with Node and npm.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {execFileSync} from 'node:child_process';

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const manifests = ['package.json', '.claude-plugin/plugin.json', ...fs.readdirSync(path.join(repo, 'npm')).map(platform => `npm/${platform}/package.json`)];
for (const file of manifests) {
  assert.equal(JSON.parse(fs.readFileSync(path.join(repo, file), 'utf8')).license, 'MIT', `${file} must declare MIT`);
}
assert.deepEqual(fs.readdirSync(repo).filter(file => /^LICENSE(?:$|[-.])/i.test(file)), ['LICENSE']);
const binary = path.resolve(process.argv[2] ?? 'target/release/ynotes');
assert.ok(fs.statSync(binary).isFile(), `Build the binary first: ${binary}`);
assert.ok(['darwin', 'linux'].includes(process.platform), 'This local smoke runner requires macOS or Linux; Windows has its own CI install job.');
const libc = process.platform === 'linux' && !process.report.getReport().header.glibcVersionRuntime ? '-musl' : '';
const platform = `${process.platform}${libc}-${process.arch}`;
const root = fs.mkdtempSync(path.join(os.tmpdir(), 'ynotes-install-'));
console.log(`Artefacts: ${root}`);
const launcher = path.join(root, 'launcher');
const native = path.join(root, 'native');
const packs = path.join(root, 'packs');
const prefix = path.join(root, 'prefix');
const fixture = path.join(root, 'example repo');
for (const dir of [launcher, native, packs, fixture]) fs.mkdirSync(dir, {recursive: true});
for (const dir of [launcher, native]) fs.mkdirSync(path.join(dir, 'bin'));
for (const file of ['package.json', 'README.md', 'RELEASING.md', 'LICENSE']) fs.copyFileSync(path.join(repo, file), path.join(launcher, file));
fs.copyFileSync(path.join(repo, 'LICENSE'), path.join(native, 'LICENSE'));
fs.copyFileSync(path.join(repo, 'bin/ynotes.js'), path.join(launcher, 'bin/ynotes.js'));
fs.copyFileSync(path.join(repo, `npm/${platform}/package.json`), path.join(native, 'package.json'));
fs.copyFileSync(binary, path.join(native, 'bin/ynotes'));
fs.chmodSync(path.join(native, 'bin/ynotes'), 0o755);
const npm = (cwd, ...args) => execFileSync('npm', [...args, '--cache', path.join(root, 'npm-cache')], {cwd, encoding: 'utf8'});
const archives = [launcher, native].map(dir => {
  const output = JSON.parse(npm(dir, 'pack', '--json', '--ignore-scripts', '--pack-destination', packs));
  const [packed] = Array.isArray(output) ? output : Object.values(output);
  if (dir === launcher) {
    const included = new Set(packed.files.map(file => file.path));
    for (const required of ['RELEASING.md', 'LICENSE']) assert.ok(included.has(required), `Packed launcher omitted ${required}`);
  }
  assert.ok(packed.files.some(file => file.path === 'LICENSE'), 'Every package must include LICENSE');
  assert.ok(!packed.files.some(file => /LICENSE[-.]/i.test(file.path)), 'Legacy project licences must not be packaged');
  return path.join(packs, packed.filename);
});
npm(root, 'install', '--global', '--prefix', prefix, '--offline', '--ignore-scripts', '--no-audit', '--no-fund', ...archives);
const installed = path.join(prefix, 'bin/ynotes');
const calls = [];
const yn = (...args) => {
  const result = JSON.parse(execFileSync(installed, [...args, '--json'], {cwd: fixture, encoding: 'utf8'}));
  assert.equal(result.success, true);
  calls.push({args, result});
  return result.data;
};
const version = execFileSync(installed, ['--version'], {encoding: 'utf8'}).trim();
assert.deepEqual(yn('files'), {store: 'absent'});
assert.equal(fs.existsSync(path.join(fixture, '.ynotes')), false);
execFileSync('git', ['init', '-q'], {cwd: fixture});
execFileSync('git', ['config', 'user.name', 'Install smoke'], {cwd: fixture});
execFileSync('git', ['config', 'user.email', 'smoke@example.invalid'], {cwd: fixture});
fs.writeFileSync(path.join(fixture, 'old.js'), 'export function retryDelay() {\n  return 100;\n}\n');
execFileSync('git', ['add', 'old.js'], {cwd: fixture});
execFileSync('git', ['commit', '-qm', 'fixture'], {cwd: fixture});
const saved = yn('save', 'old.js', '1:3', '--code', 'export function retryDelay() {', '-m', 'Demo constraint: the fixture consumer expects planned backoff, not elapsed wait.');
execFileSync('git', ['mv', 'old.js', 'new.js'], {cwd: fixture});
assert.equal(yn('files').files[0].target, 'new.js');
const recalled = yn('query', 'new.js').notes;
assert.equal(recalled.length, 1);
assert.equal(recalled[0].id, saved.id);
assert.equal(recalled[0].relocated_to, 'new.js');
const hook = execFileSync(installed, ['hook', 'session-start'], {cwd: fixture, input: JSON.stringify({cwd: fixture}), encoding: 'utf8'});
assert.ok(hook.includes('new.js'));
assert.ok(!hook.includes('old.js'));
execFileSync('git', ['commit', '-qm', 'rename fixture'], {cwd: fixture});
assert.equal(yn('reanchor').relocated.length, 1);
assert.equal(yn('query', 'new.js').notes[0].status, 'anchored');
// Extraction into an existing file requires deliberate reassignment. Verify
// the replacement before removing the historical record, as the guide says.
fs.writeFileSync(path.join(fixture, 'destination.js'), 'export function retryDelay() {\n  return 100;\n}\n');
fs.writeFileSync(path.join(fixture, 'new.js'), '// Function extracted into destination.js.\n');
const historical = yn('query', 'new.js').notes[0];
assert.equal(historical.status, 'orphaned');
assert.equal(yn('query', 'destination.js').notes.length, 0);
const replacement = yn('save', 'destination.js', '1:3', '--code', 'export function retryDelay() {', '-m', historical.body);
assert.equal(yn('query', 'destination.js').notes[0].id, replacement.id);
yn('delete', historical.id);
assert.equal(yn('query', 'new.js').notes.length, 0);
assert.equal(yn('query', 'destination.js').notes[0].body, historical.body);
assert.ok(execFileSync(installed, ['skills', 'get', 'core'], {cwd: fixture, encoding: 'utf8'}).length > 0);
yn('doctor');
fs.writeFileSync(path.join(root, 'results.json'), JSON.stringify({version, platform, binary, archives, calls}, null, 2));
console.log(`PASS ${version}: packed install, absent-store discovery, save, staged rename discovery and hook, recall, reanchor, extraction recovery, guidance and doctor.`);
