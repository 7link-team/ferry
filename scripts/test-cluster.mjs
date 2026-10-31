// Real three-server/JetStream/ordinary Runner acceptance. No installed services.
import assert from 'node:assert/strict';
import {spawn} from 'node:child_process';
import {mkdtemp,mkdir,readFile,rm,open,copyFile} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join,resolve} from 'node:path';
import {randomBytes,createHash} from 'node:crypto';
import {createServer as httpServer,request as httpRequest} from 'node:http';
import {createServer as netServer,createConnection} from 'node:net';
import {chromium} from 'playwright';

const work=await mkdtemp(join(tmpdir(),'cluster-acceptance-'));
const processes=[],logs=[],sockets=new Set();let gateway,browser,cookie='',failed=false;
const sleep=ms=>new Promise(resolve=>setTimeout(resolve,ms));
const until=async(check,label,seconds=45)=>{let last;for(let n=0;n<seconds*10;n++){try{if(await check())return;}catch(error){last=error;}await sleep(100);}throw new Error(`Timeout: ${label}; ${last?.message||''}`);};
const port=async()=>{const server=netServer();await new Promise(done=>server.listen(0,'127.0.0.1',done));const port=server.address().port;await new Promise(done=>server.close(done));return port;};
const launch=async(binary,args,env,name)=>{
  const log=await open(join(work,`${name}.log`),'w');logs.push(log);
  const child=spawn(resolve(binary),args,{env,cwd:work,stdio:['ignore',log.fd,log.fd],windowsHide:true});processes.push(child);
  child.on('error',()=>{});return child;
};
const stop=async child=>{if(child.exitCode!==null)return;child.kill();await Promise.race([new Promise(done=>child.once('exit',done)),sleep(5000)]);};
const api=async(origin,path,body,method=body===undefined?'GET':'POST',expected=200)=>{
  const response=await fetch(origin+'/api'+path,{method,headers:{'content-type':'application/json','X-Ferry-Request':'1',Cookie:cookie},...(body===undefined?{}:{body:JSON.stringify(body)}),signal:AbortSignal.timeout(40000)});
  if(response.headers.getSetCookie().length)cookie=response.headers.getSetCookie().map(value=>value.split(';')[0]).join('; ');
  const raw=await response.text();let value;try{value=raw?JSON.parse(raw):null;}catch{throw new Error(`${path}: invalid response`);}
  assert.equal(response.status,expected,`${path}: ${JSON.stringify(value)}`);return value;
};
try {
  const suffix=process.platform==='win32'?'.exe':'';
  const serverBinary=join(work,'server'+suffix),runnerBinary=join(work,'runner'+suffix);
  await copyFile(process.env.FERRY_BINARY||'target/debug/ferry-server'+suffix,serverBinary);
  await copyFile(process.env.RUNNER_BINARY||'target/debug/ferry-runner'+suffix,runnerBinary);
  const np=await port(),ports=[await port(),await port(),await port()];
  const origins=ports.map(port=>`http://127.0.0.1:${port}`);
  const namespace='test_'+randomBytes(6).toString('hex');
  const env={...process.env,FERRY_MASTER_KEY:randomBytes(32).toString('hex'),FERRY_NATS_URL:`nats://127.0.0.1:${np}`,FERRY_CLUSTER_NAMESPACE:namespace,FERRY_DATABASE_URL:process.env.FERRY_TEST_DATABASE_URL||`sqlite://${join(work,'shared.sqlite').replaceAll('\\','/')}?mode=rwc`};
  const natsArgs=['-js','-a','127.0.0.1','-p',String(np),'-sd',join(work,'jetstream')];
  let nats=await launch(process.env.NATS_SERVER_BINARY||'nats-server',natsArgs,process.env,'nats');
  await sleep(500);assert.equal(nats.exitCode,null,'NATS must start');
  const servers=[];
  for(let n=0;n<3;n++){
    const dir=join(work,`server-${n}`);await mkdir(dir);
    servers.push(await launch(serverBinary,['--data-dir',dir,'--bind',`127.0.0.1:${ports[n]}`],env,`server-${n}`));
    await until(async()=>{assert.equal(servers[n].exitCode,null,`server ${n} exited`);return(await api(origins[n],'/status')).ok;},`server ${n} startup`);
  }
  console.log('PASS three independent server processes share JetStream and database');
  const A=origins[0],B=origins[1];
  await api(A,'/setup',{name:'Cluster admin',email:'cluster@example.invalid',password:'temporary-cluster-password',token:(await readFile(join(work,'server-0','setup-token'),'utf8')).trim()});
  const ws=(await api(B,'/workspaces',{name:'Cluster',slug:'cluster'})).id;
  const host=(await api(B,`/workspaces/${ws}/nodes`,{name:'Cluster Runner',connection:'runner'},'POST',201)).id;
  const root=`/workspaces/${ws}/nodes/${host}`;
  const {token}=await api(B,`/workspaces/${ws}/runner-tokens`,{host_id:host},'POST',201);
  let controlPort=ports[0];
  const counts={control:0,sessions:0};
  gateway=httpServer((req,res)=>{
    const remote=httpRequest({host:'127.0.0.1',port:ports[1],method:req.method,path:req.url,headers:req.headers},reply=>{res.writeHead(reply.statusCode,reply.headers);reply.pipe(res);});
    remote.on('error',()=>{res.writeHead(502);res.end();});req.pipe(remote);
  });
  gateway.on('upgrade',(req,socket,head)=>{
    const control=req.url.startsWith('/api/runners/connect');counts[control?'control':'sessions']++;
    const upstream=createConnection({host:'127.0.0.1',port:control?controlPort:ports[2]});
    sockets.add(socket);sockets.add(upstream);for(const stream of [socket,upstream])stream.on('close',()=>sockets.delete(stream));
    upstream.on('connect',()=>{upstream.write(`${req.method} ${req.url} HTTP/1.1\r\n${Object.entries(req.headers).map(([key,value])=>`${key}: ${value}`).join('\r\n')}\r\n\r\n`);if(head.length)upstream.write(head);socket.pipe(upstream);upstream.pipe(socket);});
    upstream.on('error',()=>socket.destroy());socket.on('error',()=>upstream.destroy());socket.on('close',()=>upstream.destroy());upstream.on('close',()=>socket.destroy());
  });
  await new Promise(done=>gateway.listen(0,'127.0.0.1',done));
  const runner=await launch(runnerBinary,['--server',`http://127.0.0.1:${gateway.address().port}`,'--token',token,'--state-dir',join(work,'runner')],process.env,'runner');
  await until(async()=>(await api(B,root)).state==='online','Runner control on A');
  const result=await api(B,root+'/connection-tests',{});assert(result.ok);
  console.log('PASS command from B, control on A, dedicated session callback forced to C');
  await until(async()=>(await api(B,root+'/metrics?window_seconds=3600&step_seconds=5')).samples?.length>=2,'real monitor history');
  const history=await api(B,root+'/metrics?window_seconds=3600&step_seconds=5');
  assert(history.samples.length>=2);assert(history.samples.every((sample,index,all)=>!index||sample.sampled_at>all[index-1].sampled_at));
  console.log('PASS B reads actual timestamped monitor history collected on A');
  const remoteDirectory=join(work,'files').replaceAll('\\','/').replace(/^[a-zA-Z]:/,'');
  await mkdir(join(work,'files'));
  const remotePath=remoteDirectory+'/payload.bin';
  const payload=randomBytes(2*1024*1024+123);
  const upload=await fetch(B+'/api'+root+'/files/content?path='+encodeURIComponent(remotePath),{method:'PUT',headers:{Cookie:cookie,'X-Ferry-Request':'1'},body:payload,signal:AbortSignal.timeout(40000)});
  assert(upload.ok,`upload: ${await upload.text()}`);
  const download=await fetch(B+'/api'+root+'/files/content?path='+encodeURIComponent(remotePath),{headers:{Cookie:cookie},signal:AbortSignal.timeout(40000)});
  assert(download.ok);const actual=Buffer.from(await download.arrayBuffer());assert.equal(createHash('sha256').update(actual).digest('hex'),createHash('sha256').update(payload).digest('hex'));
  assert((await api(B,root+'/files?path='+encodeURIComponent(remoteDirectory))).items.some(file=>file.name==='payload.bin'));
  console.log('PASS cross-instance streaming upload/download beyond 2 MiB, exact SHA256 and directory listing');
  browser=await chromium.launch({headless:true,...(process.env.PLAYWRIGHT_CHROME_PATH?{executablePath:process.env.PLAYWRIGHT_CHROME_PATH}:{})});
  const context=await browser.newContext();await context.addCookies(cookie.split('; ').map(value=>{const at=value.indexOf('=');return{name:value.slice(0,at),value:value.slice(at+1),url:B};}));
  const page=await context.newPage();await page.goto(B+'/api/status');
  await page.evaluate(path=>{window.framesReceived='';window.terminalClosed=false;window.terminal=new WebSocket(path);window.terminal.binaryType='arraybuffer';window.terminal.onmessage=event=>{const text=typeof event.data==='string'?event.data:new TextDecoder().decode(event.data);window.framesReceived+=text;if(text.includes('\x1b[6n'))window.terminal.send(new TextEncoder().encode('\x1b[1;1R'));};window.terminal.onclose=()=>{window.terminalClosed=true;};},B.replace('http:','ws:')+'/api'+root+'/terminal-sessions');
  await until(()=>page.evaluate(()=>window.terminal.readyState===WebSocket.OPEN),'browser terminal on B');
  const command=process.platform==='win32'?`powershell -NoProfile -Command "Write-Output ('CLUSTER_' + (2+3))"\r`:'printf "CLUSTER_%s\\n" $((2+3))\r';
  await page.evaluate(command=>window.terminal.send(new TextEncoder().encode(command)),command);
  await until(()=>page.evaluate(()=>window.framesReceived.includes('CLUSTER_5')),'actual shell output');
  console.log('PASS real interactive terminal across B/A/C');
  const other=(await api(B,'/workspaces',{name:'Other',slug:'other'})).id;
  await api(B,`/workspaces/${other}/nodes/${host}/metrics?window_seconds=300&step_seconds=5`,undefined,'GET',404);
  console.log('PASS workspace isolation before cluster forwarding');
  await stop(nats);
  await until(()=>page.evaluate(()=>window.terminalClosed),'terminal closes on broker failure',35);
  nats=await launch(process.env.NATS_SERVER_BINARY||'nats-server',natsArgs,process.env,'nats-restarted');
  await until(async()=>{try{return(await api(B,root+'/connection-tests',{})).ok;}catch{return false;}},'broker restart and Runner control recovery',90);
  assert.equal(runner.exitCode,null);
  console.log('PASS broker failure closes live streams; broker restart restores commands with the same Runner identity');
  await page.evaluate(path=>{window.terminalClosed=false;window.terminal=new WebSocket(path);window.terminal.onclose=()=>{window.terminalClosed=true;};},B.replace('http:','ws:')+'/api'+root+'/terminal-sessions');
  await until(()=>page.evaluate(()=>window.terminal.readyState===WebSocket.OPEN),'new terminal after broker recovery');
  // Owner failover: only this isolated server process is stopped.
  controlPort=ports[1];await stop(servers[0]);
  await until(()=>page.evaluate(()=>window.terminalClosed),'terminal visibly closes after owner loss',35);
  await until(async()=>{try{return(await api(B,root+'/connection-tests',{})).ok;}catch{return false;}},'Runner reconnects to B after owner lease expiry',75);
  assert.equal(runner.exitCode,null);assert(counts.control>=2&&counts.sessions>=4);
  console.log('PASS owner process failure closes terminal; Runner reconnects and commands recover without affinity');
  await api(B,root+'/connection-credentials',{},'POST');
  await until(async()=>(await api(B,root)).state==='offline','cross-instance credential reset');
  console.log('PASS credential rotation disconnects the old Runner');
  console.log('PASS cluster acceptance');
} catch(error) {
  failed=true;
  console.error('Cluster acceptance failed; isolated logs:',work);throw error;
} finally {
  if(browser)await browser.close();
  for(const socket of sockets)socket.destroy();
  if(gateway)await new Promise(done=>gateway.close(done));
  for(const child of processes.reverse())await stop(child);
  for(const log of logs)await log.close();
  if(!failed&&process.env.KEEP_CLUSTER_TEST_DATA!=='1'){
    assert(resolve(work).startsWith(resolve(tmpdir())+'/')||resolve(work).startsWith(resolve(tmpdir())+'\\'));
    assert(work.includes('cluster-acceptance-'));
    await rm(work,{recursive:true,force:true});
  }
}
