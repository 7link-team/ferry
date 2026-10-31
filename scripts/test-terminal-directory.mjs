// Exercise the actual native shell, including CMD's expansion and drive rules.
import assert from 'node:assert/strict';
import { readFile, mkdtemp, mkdir, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { spawn } from 'node:child_process';
import ts from 'typescript';

const source = await readFile('src/components/HostTerminal.tsx', 'utf8');
const code = source.slice(source.indexOf('export function terminalStartCommand'), source.indexOf('type Status'));
const compiled = ts.transpileModule(code, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ES2022 } }).outputText;
const { terminalStartCommand } = await import('data:text/javascript;base64,' + Buffer.from(compiled).toString('base64'));
assert.throws(() => terminalStartCommand('windows', 'C:/bad\r\nwhoami'));
assert.equal(terminalStartCommand('linux'), '');
assert.equal(terminalStartCommand('linux', "/tmp/a'b"), "cd '/tmp/a'\\''b'\r");
assert.equal(terminalStartCommand('windows', undefined, '0.1.0-alpha.20'), '');
assert(terminalStartCommand('windows', undefined, '0.1.0-alpha.19').includes('EncodedCommand'));

if (process.platform === 'win32') {
  const root = await mkdtemp(join(tmpdir(), 'terminal-directory-'));
  const run = (command, env = process.env) => new Promise((resolve, reject) => {
    const child = spawn('cmd.exe', ['/d', '/q'], { windowsHide: true, env, stdio: 'pipe' });
    let output = '';
    const timer = setTimeout(() => { child.kill(); reject(new Error('CMD timed out')); }, 20000);
    child.stdout.on('data', chunk => { output += chunk.toString('utf8'); });
    child.stderr.on('data', chunk => { output += chunk.toString('utf8'); });
    child.on('error', reject);
    child.on('close', status => { clearTimeout(timer); assert.equal(status, 0, output); resolve(output); });
    child.stdin.end(command.replaceAll('\r', '\r\n') + '@cd\r\nexit\r\n');
  });
  try {
    for (const name of ['normal', "space ' quote", '%USERPROFILE% & (special)!', '\u4e2d\u6587']) {
      const path = join(root, name); await mkdir(path);
      console.log('Testing native directory:', name);
      const output = await run(terminalStartCommand('windows', path.replaceAll('\\', '/')));
      assert(output.includes('>' + path + '\r\n'), output);
    }
    const home = await run(terminalStartCommand('windows', undefined, '0.1.0-alpha.19'), { ...process.env, USERPROFILE: root });
    assert(home.includes('>' + root + '\r\n'), home);
    const serviceHome = await run(terminalStartCommand('windows', undefined, '0.1.0-alpha.19'), { ...process.env, USERPROFILE: 'C:\\Windows\\System32\\config\\systemprofile' });
    assert(!serviceHome.toLowerCase().includes('system32\\config\\systemprofile>'), serviceHome);
    assert(serviceHome.includes('>C:\\Users\\'), serviceHome);
    assert(terminalStartCommand('', 'D:/logs').includes('pushd'));
    console.log('PASS native CMD literal directories: spaces, apostrophe, environment-like names, metacharacters, Unicode; legacy home; platform inference');
  } finally { await rm(root, { recursive: true, force: true }); }
}
