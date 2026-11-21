// Isolated Server + real Runner resource-policy acceptance. No installed service
// or existing workspace is touched. Build Server/Runner and frontend first.
import assert from 'node:assert/strict';
import {spawn, execFile} from 'node:child_process';
import {createHash, randomBytes} from 'node:crypto';
import {open, mkdir, mkdtemp, readdir, readFile, writeFile} from 'node:fs/promises';
import http from 'node:http';
import {createServer as netServer} from 'node:net';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {Readable} from 'node:stream';
import {promisify} from 'node:util';

const exec = promisify(execFile);
const work = await mkdtemp(join(tmpdir(), 'resource-policy-runtime-'));
const processes = [], logs = [];
let upstream, cookie = '', monitoring = false;
const checks = {};
const sleep = ms => new Promise(done => setTimeout(done, ms));
const port = () => new Promise(done => {
  const listener = netServer();
  listener.listen(0, '127.0.0.1', () => {
    const value = listener.address().port;
    listener.close(() => done(value));
  });
});
async function launch(name, binary, args, extra = {}) {
  const log = await open(join(work, `${name}.log`), 'w'); logs.push(log);
  const env = {...Object.fromEntries(Object.entries(process.env).filter(([key]) => !key.startsWith('FERRY_'))), ...extra};
  const child = spawn(resolve(binary), args, {env, stdio:['ignore', log.fd, log.fd], windowsHide:true});
  processes.push(child);
  child.on('error', error => { child.launchError = error; });
  return child;
}
async function until(name, probe, seconds = 60) {
  let last;
  for (let i = 0; i < seconds * 10; i++) {
    try { const result = await probe(); if (result) return result; } catch (error) { last = error; }
    await sleep(100);
  }
  throw Error(`${name} did not become ready: ${last?.message ?? ''}`);
}
const serverPort = await port(), base = `http://127.0.0.1:${serverPort}`;
async function api(path, body, method = body === undefined ? 'GET' : 'POST') {
  const response = await fetch(base + path, {method, headers:{'content-type':'application/json', 'x-ferry-request':'1', cookie}, ...(body === undefined ? {} : {body:JSON.stringify(body)}), signal:AbortSignal.timeout(15000)});
  const cookies = response.headers.getSetCookie();
  if (cookies.length) cookie = cookies.map(value => value.split(';')[0]).join('; ');
  const text = await response.text();
  assert.equal(response.ok, true, `Isolated fixture API ${method} ${path}: ${response.status} ${text.slice(0,300)}`);
  return text ? JSON.parse(text) : null;
}
async function memory(pids) {
  if (process.platform === 'win32') {
    const command = `Get-Process -Id ${pids.join(',')} | Select-Object Id,WorkingSet64 | ConvertTo-Json -Compress`;
    const {stdout} = await exec('powershell.exe', ['-NoProfile','-NonInteractive','-Command',command], {windowsHide:true});
    const rows = JSON.parse(stdout);
    return Object.fromEntries((Array.isArray(rows) ? rows : [rows]).map(row => [row.Id, row.WorkingSet64]));
  }
  const {stdout} = await exec('ps', ['-o','pid=,rss=','-p',pids.join(',')], {windowsHide:true});
  return Object.fromEntries(stdout.trim().split('\n').map(row => { const [pid, rss] = row.trim().split(/\s+/); return [pid, Number(rss)*1024]; }));
}
const block = Buffer.alloc(64 * 1024, 91);
const totalBytes = 1024 * 1024 * 1024 + 65537;
function* blocks() {
  for (let remaining = totalBytes; remaining > 0; remaining -= block.length) yield block.subarray(0, Math.min(block.length, remaining));
}
const expectedHash = createHash('sha256'); for (const chunk of blocks()) expectedHash.update(chunk);
const expected = expectedHash.digest('hex');
function previewRequest(url, method = 'GET', body) {
  const target = new URL(url);
  return new Promise((done, fail) => {
    const request = http.request({hostname:'127.0.0.1', port:serverPort, path:target.pathname + target.search, method, headers:{host:target.host}}, done);
    request.on('error', fail);
    request.setTimeout(30000, () => request.destroy(Error('No preview transfer progress for 30 seconds')));
    if (body) body.on('error', fail).pipe(request); else request.end();
  });
}

try {
  const suffix = process.platform === 'win32' ? '.exe' : '';
  const data = join(work, 'data'), temp = join(work,'temp'); await mkdir(temp);
  const server = await launch('server', process.env.FERRY_BINARY || `target/debug/ferry-server${suffix}`, [], {FERRY_DATA_DIR:data, FERRY_BIND:`127.0.0.1:${serverPort}`, FERRY_DATABASE_URL:'', FERRY_NATS_URL:'', FERRY_REDIS_URL:'',TMP:temp,TEMP:temp});
  await until('Server', async () => (await api('/api/status')).ok);
  const password = randomBytes(24).toString('hex');
  await api('/api/setup', {name:'Resource policy fixture', email:'resource-fixture@example.invalid', password, token:(await readFile(join(data,'setup-token'),'utf8')).trim()});
  if (!cookie) await api('/api/sessions', {email:'resource-fixture@example.invalid',password});
  const configure = async resource_limits => {
    const current = await api('/api/platform/settings/1');
    return api('/api/platform/settings/1', {version:current.version, registration_enabled:current.registration_enabled, public_url:base, preview_domain:'preview.test', resource_limits}, 'PATCH');
  };
  await configure({});
  const initial = await api('/api/platform/settings/1');
  assert(Object.values(initial.resource_limits).every(value => value === null));
  checks.defaultPolicy = 'all null';
  const workspace = (await api('/api/workspaces', {name:'Resource fixture',slug:'resource-fixture'})).id;
  const root = `/api/workspaces/${workspace}`;
  const host = (await api(root+'/nodes', {name:'Isolated resource Runner',connection:'runner'})).id;
  const token = (await api(root+'/runner-tokens', {host_id:host})).token;
  // A disposable CLI protocol fixture exercises real Runner file collection.
  // It is only discoverable in this child Runner's PATH; no installed CLI changes.
  const fixturePath = join(work,'cli'), packagePath = join(fixturePath,'node_modules','@anthropic-ai','claude-code');
  await mkdir(packagePath,{recursive:true});
  await writeFile(join(packagePath,'package.json'),JSON.stringify({bin:{claude:'fixture.cjs'}}));
  await writeFile(join(packagePath,'fixture.cjs'),`
const fs=require('node:fs'), readline=require('node:readline');
if(process.argv.includes('--version')) { console.log('2.1.0'); process.exit(0); }
if(process.argv.includes('auth')) { console.log(JSON.stringify({loggedIn:true})); process.exit(0); }
const send=value=>process.stdout.write(JSON.stringify(value)+'\\n');
const lines=readline.createInterface({input:process.stdin});
lines.on('line',line=>{const value=JSON.parse(line);
 if(value.type==='control_request') send({type:'control_response',response:{request_id:'initialize',subtype:'success'}});
 if(value.type==='user') {
  fs.mkdirSync('output',{recursive:true}); const fd=fs.openSync('output/pressure.txt','w');
  const block=Buffer.alloc(65536,92); for(let left=${totalBytes};left>0;left-=block.length)fs.writeSync(fd,block,0,Math.min(left,block.length)); fs.closeSync(fd);
  send({type:'system',subtype:'init',session_id:'fixture-session'});
  send({type:'assistant',session_id:'fixture-session',message:{id:'final',content:[{type:'text',text:'done'}]}});
  send({type:'result',session_id:'fixture-session',subtype:'success',is_error:false,usage:{input_tokens:1,output_tokens:1}});
  lines.close(); process.stdin.destroy();
 }
});
`);
  const runner = await launch('runner', process.env.FERRY_RUNNER_BINARY || `target/debug/ferry-runner${suffix}`, ['--server',base,'--token',token,'--state-dir',join(work,'runner')], {PATH:fixturePath + (process.platform === 'win32' ? ';' : ':') + process.env.PATH});
  await until('Runner', async () => (await api(root+'/nodes/'+host)).state === 'online');
  const owner = (await api(root+'/members')).items.find(row=>row.email==='resource-fixture@example.invalid');
  const model = await api(root+'/ai-models',{name:'Fixture CLI',kind:'cli',runtime:'claude',models:[],default_model:'',state:'enabled'});
  const cwd = join(work,'agent-cwd'); await mkdir(cwd);
  const agent = await api(root+'/ai-agents',{name:'File fixture',runtime:'claude',model_profile_id:model.id,host_id:host,owner_id:owner.user_id,cwd,permission:'read_only',visibility:'workspace',state:'enabled'});
  await until('CLI probe',async()=>{await api(root+`/ai-agents/${agent.id}/probes`,{});return true;});
  const conversation = await api(root+'/conversations',{agent_id:agent.id,cwd,permission:'read_only',title:'File upload fixture'});
  const definition={engine:'ferry-graph',schema:1,settings:{timezone:'UTC'},nodes:[
    {id:'start',name:'Start',type:'core.trigger.manual',type_version:1,params:{input_schema:{type:'object'}},position:[0,0]},
    {id:'agent',name:'Agent',type:'ai.agent.run',type_version:1,params:{agent_id:agent.id,prompt:'Create the fixture file.',files:{enabled:true,formats:['txt']}},position:[300,0]}
  ],connections:[{from:{node:'start',port:'main',index:0},to:{node:'agent',port:'main',index:0}}]};
  const workflow=await api(root+'/workflows',{name:'File streaming fixture',definition:JSON.stringify(definition)});
  const capability = await api(root+`/nodes/${host}/runner-updates`);
  assert.equal(capability.supports_resource_policy, true);
  checks.runnerPolicyCapability = true;
  let active = 0, peak = 0;
  upstream = http.createServer(async (request, response) => {
    try {
      if (request.url === '/download') {
        response.writeHead(200, {'content-type':'application/octet-stream'});
        Readable.from(blocks()).pipe(response);
      } else if (request.url === '/upload') {
        let size = 0; const hash = createHash('sha256');
        for await (const chunk of request) { size += chunk.length; hash.update(chunk); }
        response.writeHead(200, {'content-type':'application/json'});
        response.end(JSON.stringify({bytes:size, sha256:hash.digest('hex')}));
      } else if (request.url === '/slow') {
        active++; peak = Math.max(peak, active);
        await sleep(500); active--; response.end('ok');
      } else response.end('ok');
    } catch { response.destroy(); }
  });
  await new Promise(done => upstream.listen(0,'127.0.0.1',done));
  const hole = await api(root+'/holes', {name:'Resource transfer fixture',host_id:host,target_port:upstream.address().port,ttl_hours:1});
  const url = path => new URL(path, hole.url).toString();
  const baseline = await memory([server.pid,runner.pid]); const peaks = {...baseline};
  let monitorFailure, onlineChecks = 0;
  monitoring = true;
  const monitor = (async () => {
    while (monitoring) {
      try {
        const current = await memory([server.pid,runner.pid]);
        for (const [pid,value] of Object.entries(current)) peaks[pid] = Math.max(peaks[pid] ?? 0,value);
        assert.equal((await api(root+'/nodes/'+host)).state,'online'); onlineChecks++;
      } catch (error) { monitorFailure = error; }
      await sleep(500);
    }
  })();
  let metadataWrites=0;
  const writeProgress=(async()=>{try{while(monitoring){await api(root+'/workflows',{name:'Concurrent metadata '+metadataWrites,definition:JSON.stringify({engine:'ferry-graph',schema:1,settings:{timezone:'UTC'},nodes:[{id:'start',name:'Start',type:'core.trigger.manual',type_version:1,params:{input_schema:{type:'object'}},position:[0,0]}],connections:[]})});metadataWrites++;await sleep(500);}}catch(error){monitorFailure=error;}})();
  function fileRequest(path,method='GET',headers={},body){
    return new Promise((done,fail)=>{
      const request=http.request(base+path,{method,headers:{cookie,...headers}},done);
      request.on('error',fail);
      // Native HTTP has no hidden Undici five-minute response-header deadline.
      request.setTimeout(30*60*1000,()=>request.destroy(Error('File transfer made no progress for 30 minutes')));
      if(body){body.on('error',error=>request.destroy(error));body.pipe(request);}else request.end();
    });
  }
  async function multipart(path){
    const boundary='fixture-'+randomBytes(12).toString('hex');
    const prefix=Buffer.from('--'+boundary+'\r\nContent-Disposition: form-data; name="file"; filename="pressure.txt"\r\nContent-Type: text/plain\r\n\r\n');
    async function* body(){yield prefix;for(const chunk of blocks()){yield chunk;}yield Buffer.from('\r\n--'+boundary+'--\r\n');}
    const response=await fileRequest(path,'POST',{'content-type':'multipart/form-data; boundary='+boundary,'x-ferry-request':'1'},Readable.from(body()));
    const data=[];for await(const chunk of response)data.push(chunk);
    const text=Buffer.concat(data).toString('utf8');assert.equal(response.statusCode,201,text.slice(0,300));return JSON.parse(text);
  }
  async function verifyDownload(path,expectedHash=expected){
    const response=await fileRequest(path);assert.equal(response.statusCode,200);
    const hash=createHash('sha256');let size=0;for await(const chunk of response){size+=chunk.length;hash.update(chunk);}
    assert.equal(size,totalBytes);assert.equal(hash.digest('hex'),expectedHash);return {bytes:size,sha256:expectedHash};
  }
  const temps=async()=>(await readdir(temp)).filter(name=>name.startsWith('content-asset-')).sort();
  const beforeAbort=await temps();
  const abort=new AbortController(), boundary='cancelled-upload';
  async function* interrupted(){
    yield Buffer.from('--'+boundary+'\r\nContent-Disposition: form-data; name="file"; filename="cancelled.txt"\r\nContent-Type: text/plain\r\n\r\n');
    yield block;
    await until("active upload tempfile",async()=>(await temps()).length>beforeAbort.length,10);
    await sleep(100); abort.abort();
  }
  await fetch(base+root+`/conversations/${conversation.id}/files`,{method:'POST',headers:{cookie,'x-ferry-request':'1','content-type':'multipart/form-data; boundary='+boundary},body:Readable.from(interrupted()),duplex:'half',signal:abort.signal}).catch(error=>assert(abort.signal.aborted,error.message));
  await until('cancelled upload cleanup',async()=>JSON.stringify(await temps())===JSON.stringify(beforeAbort));
  checks.cancelledUploadTempCleanup=true;
  const conversationFile=await multipart(root+`/conversations/${conversation.id}/files`);
  checks.conversationFile=await verifyDownload(root+`/conversations/${conversation.id}/files/${conversationFile.file_id}`);
  const workflowFile=await multipart(root+`/workflows/${workflow.id}/assets`);
  checks.workflowFile=await verifyDownload(root+`/workflows/${workflow.id}/assets/${workflowFile.asset.slice(7)}`);
  const generated=await api(root+`/workflows/${workflow.id}/executions`,{mode:'manual',revision:'draft',version:workflow.version,entry_node_id:'start',inputs:{}});
  const generatedRun=await until('generated file run',async()=>{const run=await api(root+`/workflow-executions/${generated.id}`);return ['succeeded','failed','cancelled'].includes(run.state)?run:false;},1800);
  assert.equal(generatedRun.state,'succeeded',JSON.stringify(generatedRun));
  const nodes=(await api(root+`/workflow-executions/${generated.id}/node-runs`)).items;
  const node=nodes.find(row=>row.node_id==='agent');
  const nodeData=await api(root+`/workflow-executions/${generated.id}/node-runs/${node.id}/data`);
  const binary=Object.values(nodeData.output.ports[0].items[0].binary)[0];assert(binary);
  const generatedHash=createHash('sha256'); const generatedBlock=Buffer.alloc(block.length,92);
  for(let left=totalBytes;left>0;left-=generatedBlock.length)generatedHash.update(generatedBlock.subarray(0,Math.min(left,generatedBlock.length)));
  checks.generatedFile=await verifyDownload(root+`/workflows/${workflow.id}/assets/${binary.asset.slice(7)}`,generatedHash.digest('hex'));
  const download = await previewRequest(url('/download'));
  assert.equal(download.statusCode,200);
  let received = 0; const digest = createHash('sha256');
  for await (const chunk of download) { received += chunk.length; digest.update(chunk); }
  assert.equal(received,totalBytes); assert.equal(digest.digest('hex'),expected);
  checks.download = {bytes:received,sha256:expected};
  const upload = await previewRequest(url('/upload'),'POST',Readable.from(blocks()));
  assert.equal(upload.statusCode,200);
  const response = []; for await (const chunk of upload) response.push(chunk);
  assert.deepEqual(JSON.parse(Buffer.concat(response)), {bytes:totalBytes,sha256:expected});
  checks.upload = {bytes:totalBytes,sha256:expected};
  await Promise.all(Array.from({length:64}, async () => { const response = await previewRequest(url('/slow')); assert.equal(response.statusCode,200); for await (const _chunk of response) {} }));
  assert(peak > 32, `Default preview concurrency must exceed legacy 32, observed ${peak}`);
  checks.defaultBurstPeak = peak;
  await configure({preview_connections_per_entry:2}); peak = 0;
  const boundedHole = await api(root+'/holes', {name:'Configured admission fixture',host_id:host,target_port:upstream.address().port,ttl_hours:1});
  const boundedUrl = new URL('/slow',boundedHole.url).toString();
  await Promise.all(Array.from({length:6}, async () => { const response = await previewRequest(boundedUrl); assert.equal(response.statusCode,200); for await (const _chunk of response) {} }));
  assert.equal(peak,2); checks.configuredBurstPeak = peak;
  await configure({http_body_bytes:1});
  await configure({http_body_bytes:null,preview_connections_per_entry:null});
  checks.adminRecovery = true;
  monitoring = false; await monitor; await writeProgress;
  assert(metadataWrites>3);checks.concurrentMetadataWrites=metadataWrites;
  if (monitorFailure) throw monitorFailure;
  assert(onlineChecks > 0);
  const deltas = Object.fromEntries(Object.keys(baseline).map(pid => [pid,peaks[pid]-baseline[pid]]));
  for (const bytes of Object.values(deltas)) assert(bytes < 128*1024*1024, `Streaming transfer retained excessive working memory: ${bytes}`);
  checks.workingSetGrowthBytes = deltas; checks.runnerOnlineChecks = onlineChecks;
  await writeFile(join(work,'result.json'),JSON.stringify({result:'PASS',checks},null,2));
  console.log(JSON.stringify({result:'PASS',evidence:work,checks}));
} catch (error) {
  await writeFile(join(work,'result.json'),JSON.stringify({result:'FAIL',checks,error:error.stack},null,2));
  console.error(error); console.error('Evidence:',work); process.exitCode = 1;
} finally {
  monitoring = false;
  upstream?.closeAllConnections(); upstream?.close();
  for (const child of processes.reverse()) if (child.exitCode === null) child.kill();
  await Promise.all(processes.map(child => child.exitCode !== null ? undefined : Promise.race([new Promise(done => child.once('exit',done)),sleep(5000)])));
  await Promise.all(logs.map(log => log.close()));
}
