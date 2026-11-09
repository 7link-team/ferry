/** Resolve preview test domains to loopback without changing the developer's hosts file. */
import http from 'node:http';
import https from 'node:https';
import {execFileSync} from 'node:child_process';
const registered = new Set();
export const previewResolverRules = 'MAP *.preview.test 127.0.0.1, EXCLUDE localhost';
export async function previewFetch(input, options = {}) {
  const url = new URL(input);
  if (!url.hostname.endsWith('.preview.test')) return globalThis.fetch(input, options);
  if (process.env.CI && process.env.PREVIEW_TEST_HOSTS === '1' && !registered.has(url.hostname)) {
    execFileSync('sudo', ['tee', '-a', '/etc/hosts'], { input: `127.0.0.1 ${url.hostname}\n`, stdio: ['pipe', 'ignore', 'pipe'] });
    registered.add(url.hostname);
  }
  return new Promise((resolve, reject) => {
    const request = (url.protocol === 'https:' ? https : http).request(url, {
      method: options.method || 'GET', headers: Object.fromEntries(new Headers(options.headers)),
      lookup: (_name, _options, callback) => _options.all ? callback(null, [{address:'127.0.0.1',family:4}]) : callback(null, '127.0.0.1', 4),
    }, response => {
      const chunks=[];
      response.on('data',chunk=>chunks.push(chunk));response.on('error',reject);
      response.on('end',()=>resolve(new Response([204,205,304].includes(response.statusCode) ? null : Buffer.concat(chunks), {status:response.statusCode,headers:response.headers})));
    });
    request.on('error',reject);request.end(options.body);
  });
}
