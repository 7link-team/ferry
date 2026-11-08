// Real three-server/JetStream/ordinary Runner acceptance. No installed services.
import assert from 'node:assert/strict';
import {spawn} from 'node:child_process';
import {mkdtemp,mkdir,readFile,writeFile,rm,open,copyFile} from 'node:fs/promises';
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
  child.on('error',error=>{child.launchError=error;});return child;
};
const stop=async child=>{if(child.exitCode!==null)return;child.kill();await Promise.race([new Promise(done=>child.once('exit',done)),sleep(5000)]);};
const api=async(origin,path,body,method=body===undefined?'GET':'POST',expected=200)=>{
  const response=await fetch(origin+'/api'+path,{method,headers:{'content-type':'application/json','X-Ferry-Request':'1',Cookie:cookie},...(body===undefined?{}:{body:JSON.stringify(body)}),signal:AbortSignal.timeout(40000)});
  if(response.headers.getSetCookie().length)cookie=response.headers.getSetCookie().map(value=>value.split(';')[0]).join('; ');
  const raw=await response.text();let value;try{value=raw?JSON.parse(raw):null;}catch{throw new Error(`${path}: invalid response`);}
  assert.equal(response.status,expected,`${path}: ${JSON.stringify(value)}`);return value;
};
const dependency=async(component,operation)=>{
  const command=JSON.parse(process.env.FERRY_TEST_DEPENDENCY_CONTROL||'null');
  assert(Array.isArray(command)&&command.length&&command.every(part=>typeof part==='string'),'isolated dependency control command required');
  const child=spawn(command[0],[...command.slice(1),component,operation],{stdio:'inherit',windowsHide:true});
  await new Promise((done,reject)=>{child.once('error',reject);child.once('exit',code=>code===0?done():reject(new Error(`Dependency ${component} ${operation} failed: ${code}`)));});
};
try {
  const suffix=process.platform==='win32'?'.exe':'';
  const serverBinary=join(work,'server'+suffix),runnerBinary=join(work,'runner'+suffix);
  await copyFile(process.env.FERRY_BINARY||'target/debug/ferry-server'+suffix,serverBinary);
  await copyFile(process.env.RUNNER_BINARY||'target/debug/ferry-runner'+suffix,runnerBinary);
  const natsCount=Number(process.env.FERRY_TEST_NATS_NODES||1);
  assert([1,3].includes(natsCount),'test supports one or three NATS nodes');
  const brokerPorts=await Promise.all(Array.from({length:natsCount},()=>port()));
  const monitorPorts=await Promise.all(Array.from({length:natsCount},()=>port()));
  const routePorts=await Promise.all(Array.from({length:natsCount},()=>port()));
  const np=brokerPorts[0],ports=[await port(),await port(),await port()];
  const origins=ports.map(port=>`http://127.0.0.1:${port}`);
  const namespace='test_'+randomBytes(6).toString('hex');
  const env={...process.env,FERRY_MASTER_KEY:randomBytes(32).toString('hex'),FERRY_NATS_URL:`nats://127.0.0.1:${np}`,FERRY_NATS_REPLICAS:String(natsCount),FERRY_CLUSTER_NAMESPACE:namespace,FERRY_DATABASE_URL:process.env.FERRY_TEST_DATABASE_URL||`sqlite://${join(work,'shared.sqlite').replaceAll('\\','/')}?mode=rwc`};
  const brokers=[],brokerConfigs=[];
  const startBroker=async(index,label)=>launch(process.env.NATS_SERVER_BINARY||'nats-server',['-c',brokerConfigs[index]],process.env,`nats-${index}${label||''}`);
  for(let n=0;n<natsCount;n++){
    const config={server_name:`broker-${n}`,host:'127.0.0.1',port:brokerPorts[n],http:`127.0.0.1:${monitorPorts[n]}`,jetstream:{store_dir:join(work,`jetstream-${n}`)}};
    if(natsCount===3)config.cluster={name:namespace,host:'127.0.0.1',port:routePorts[n],routes:routePorts.filter((_,i)=>i!==n).map(p=>`nats://127.0.0.1:${p}`)};
    const path=join(work,`nats-${n}.json`);await writeFile(path,JSON.stringify(config));brokerConfigs.push(path);
    brokers.push(await startBroker(n));
  }
  const monitor=async(index,path)=>(await fetch(`http://127.0.0.1:${monitorPorts[index]}${path}`,{signal:AbortSignal.timeout(2000)})).json();
  await until(async()=>{
    for(let n=0;n<natsCount;n++){
      assert.equal(brokers[n].exitCode,null,`NATS ${n} must start`);
      if(!(await monitor(n,'/healthz')).status?.includes('ok'))return false;
      if(natsCount===3&&(await monitor(n,'/routez')).num_routes<2)return false;
    }return true;
  },'NATS private routes and health',60);
  const streams=async()=>{
    const items=[];
    for(let n=0;n<natsCount;n++){
      if(brokers[n].exitCode!==null||brokers[n].signalCode!==null)continue;
      const detail=await monitor(n,'/jsz?accounts=true&streams=true&config=true');
      for(const account of detail.account_details||[])for(const stream of account.stream_detail||[])if(stream.name.startsWith(`KV_${namespace}_`))items.push(stream);
    }return items;
  };
  const replicated=async()=>{
    const items=await streams();
    return ['owners','sessions','coordination'].every(suffix=>items.some(s=>s.name===`KV_${namespace}_${suffix}`&&s.config?.num_replicas===3&&s.cluster?.leader&&(s.cluster.replicas||[]).filter(r=>r.current&&!r.offline).length===2));
  };
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
  const runner=await launch(runnerBinary,['--server',`http://127.0.0.1:${gateway.address().port}`,'--token',token,'--state-dir',join(work,'runner-state')],process.env,'runner');
  await until(async()=>{if(runner.launchError)throw runner.launchError;assert.equal(runner.exitCode,null,'Runner exited before connecting');return(await api(B,root)).state==='online';},'Runner control on A');
  const result=await api(B,root+'/connection-tests',{});assert(result.ok);
  console.log('PASS command from B, control on A, dedicated session callback forced to C');
  if(natsCount===3){
    await until(replicated,'three replicas of all Ferry KV buckets are current',90);
    const owners=(await streams()).find(s=>s.name===`KV_${namespace}_owners`&&s.cluster?.leader);
    const leader=Number(owners.cluster.leader.replace('broker-',''));
    assert(Number.isInteger(leader)&&leader>=0&&leader<3);
    await stop(brokers[leader]);
    await until(async()=>{
      const items=await streams();
      return items.some(s=>s.name===owners.name&&s.cluster?.leader&&s.cluster.leader!==owners.cluster.leader)&&(await api(B,root+'/connection-tests',{})).ok;
    },'Ferry commands recover through discovered peers after KV leader exits',90);
    assert.equal(runner.exitCode,null);
    brokers[leader]=await startBroker(leader,'-returned');
    await until(replicated,'returned NATS member catches up all three Ferry KV buckets',90);
    console.log('PASS three-node JetStream leader failure, real cross-instance command recovery and original-store replica catch-up');
  }
  if(process.env.FERRY_TEST_CLUSTER_EXECUTIONS==='1'){
    const workspaceRoot=`/workspaces/${ws}`;
    let limit=(await api(B,workspaceRoot+'/settings')).items.find(row=>row.key==='pipeline_concurrent_runs');
    assert(limit,'workspace concurrency setting');
    limit=await api(B,workspaceRoot+'/settings/'+limit.id,{version:limit.version,value:1},'PATCH');
    const marker=join(work,'execution-markers.txt').replaceAll('\\','/');
    const command=(label,hold)=>process.platform==='win32'
      ? `Add-Content -LiteralPath '${marker.replaceAll("'","''")}' -Value '${label}'; Write-Output ${label}_RUN_MARKER; Start-Sleep -Seconds ${hold}`
      : `printf '${label}\\n' >> '${marker.replaceAll("'","'\\''")}'; printf '${label}_RUN_MARKER\\n'; sleep ${hold}`;
    const pipeline=async(label,hold)=>(await api(B,workspaceRoot+'/pipelines',{
      name:`Cluster ownership ${label}`,definition_format:'github-actions',
      definition:JSON.stringify({on:'workflow_dispatch',jobs:{build:{'runs-on':'self-hosted',steps:[{run:command(label,hold)}]}}}),
      executor_bindings:[{job_id:'build',target_kind:'host',target_ref:host,image:'host'}],
    },'POST',201)).id;
    const start=async pipeline=>(await api(B,`${workspaceRoot}/pipelines/${pipeline}/runs`,{},'POST',201)).id;
    const state=id=>api(B,`${workspaceRoot}/pipeline-runs/${id}`);
    const output=async id=>(await api(B,`${workspaceRoot}/pipeline-runs/${id}/logs?after=0`)).items.map(row=>row.chunk).join('');
    const first=await start(await pipeline('FIRST',90));
    await until(async()=>(await output(first)).includes('FIRST_RUN_MARKER'),'first real Actions command',90);
    const second=await start(await pipeline('SECOND',0));
    // Starting another scheduler while a lease is live must not sweep the run.
    const fourthPort=await port(),fourthDir=join(work,'server-3');await mkdir(fourthDir);
    const fourth=await launch(serverBinary,['--data-dir',fourthDir,'--bind',`127.0.0.1:${fourthPort}`],env,'server-3');
    servers.push(fourth);
    await until(async()=>{assert.equal(fourth.exitCode,null);return(await api(`http://127.0.0.1:${fourthPort}`,'/status')).ok;},'fourth scheduler startup');
    for(let n=0;n<32;n++){
      const [one,two]=await Promise.all([state(first),state(second)]);
      assert.equal(one.state,'running','a live leased run survives other scheduler startups and renews beyond 30s TTL');
      assert.equal(two.state,'queued','the workspace slot is global across all scheduler processes');
      await sleep(1000);
    }
    console.log('PASS four schedulers respect one workspace slot beyond lease TTL; startup preserves live ownership');
    await api(B,`${workspaceRoot}/pipeline-runs/${first}/cancellations`,{});
    await until(async()=>(await state(first)).state==='cancelled','first remote command cancellation',90);
    await until(async()=>(await state(second)).state==='succeeded','queued command starts after cleanup',90);
    assert.deepEqual((await readFile(marker,'utf8')).trim().split(/\r?\n/),['FIRST','SECOND'],'both remote side effects occur exactly once');
    assert((await output(second)).includes('SECOND_RUN_MARKER'));
    limit=await api(B,workspaceRoot+'/settings/'+limit.id,{version:limit.version,value:10},'PATCH');
    console.log('PASS cross-instance cancellation waits for remote cleanup and admits the next command exactly once');
  }
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
  if(process.env.FERRY_TEST_CLUSTER_DEPENDENCIES==='1'){
    assert.equal(env.FERRY_ARTIFACT_STORE,'s3','dependency acceptance requires actual S3');
    assert.match(env.FERRY_DATABASE_URL,/^(postgres|mysql):\/\//,'dependency acceptance requires shared SQL');
    const graph={engine:'ferry-graph',schema:1,nodes:[{id:'start',name:'start',type:'core.trigger.manual',type_version:1,params:{input_schema:{type:'object',properties:{}}},position:[0,0]}],connections:[]};
    const workflow=(await api(A,`/workspaces/${ws}/workflows`,{name:'Cluster shared S3',definition:JSON.stringify(graph)},'POST',201)).id;
    const assetRoot=`/workspaces/${ws}/workflows/${workflow}/assets`;
    const uploadAsset=async(origin,bytes,expected=201)=>{
      const data=new FormData();data.append('file',new Blob([bytes],{type:'application/octet-stream'}),'cluster.bin');
      const response=await fetch(origin+'/api'+assetRoot,{method:'POST',headers:{Cookie:cookie,'X-Ferry-Request':'1'},body:data,signal:AbortSignal.timeout(90000)});
      const body=await response.text();assert.equal(response.status,expected,body);return JSON.parse(body);
    };
    const assetBytes=randomBytes(128*1024+17),asset=await uploadAsset(A,assetBytes);
    const assetPath=assetRoot+'/'+asset.asset.slice(7);
    const readAsset=async(origin,expected=200)=>{
      const response=await fetch(origin+'/api'+assetPath,{headers:{Cookie:cookie},signal:AbortSignal.timeout(90000)});
      assert.equal(response.status,expected);const data=Buffer.from(await response.arrayBuffer());
      if(expected===200)assert.deepEqual(data,assetBytes);return data;
    };
    await readAsset(origins[2]);
    const range=await fetch(B+'/api'+assetPath,{headers:{Cookie:cookie,Range:'bytes=0-63'},signal:AbortSignal.timeout(40000)});
    assert.equal(range.status,206);assert.deepEqual(Buffer.from(await range.arrayBuffer()),assetBytes.subarray(0,64));
    await api(B,`/workspaces/${other}/workflows/${workflow}/assets/${asset.asset.slice(7)}`,undefined,'GET',404);
    console.log('PASS real S3 upload on A, byte-exact download on C, range on B and cross-workspace rejection');
    const retryBytes=randomBytes(777);
    try{
      await dependency('s3','stop');
      await readAsset(B,409);
      await uploadAsset(B,retryBytes,409);
    }finally{await dependency('s3','start');}
    await readAsset(origins[2]);
    const retried=await uploadAsset(B,retryBytes);
    assert.equal(retried.asset,'sha256:'+createHash('sha256').update(retryBytes).digest('hex'));
    console.log('PASS S3 outage reports failure; same-store recovery preserves bytes and failed upload can be retried');
    const rejectedSlug='sql-outage-'+randomBytes(5).toString('hex');
    try{
      await dependency('database','stop');
      for(const origin of [A,B]){
        const response=await fetch(origin+`/api/workspaces/${ws}/nodes/${host}`,{headers:{Cookie:cookie},signal:AbortSignal.timeout(45000)});
        await response.arrayBuffer();assert(response.status>=500&&response.status<600,'SQL outage must not return fabricated successful data');
      }
      const rejected=await fetch(B+'/api/workspaces',{method:'POST',headers:{Cookie:cookie,'content-type':'application/json','X-Ferry-Request':'1'},body:JSON.stringify({name:'Rejected during SQL outage',slug:rejectedSlug}),signal:AbortSignal.timeout(45000)});
      await rejected.arrayBuffer();assert(rejected.status>=500&&rejected.status<600,'SQL outage must not report a successful write');
    }finally{await dependency('database','start');}
    await until(async()=>(await api(B,root)).id===host,'pooled SQL connections recover with original identity',90);
    assert(!(await api(B,'/me')).workspaces.some(row=>row.slug===rejectedSlug),'rejected write must not appear after recovery');
    await until(async()=>{assert.equal(runner.exitCode,null,'Runner must keep retrying after a transient database outage');return(await api(B,root+'/connection-tests',{})).ok;},'Runner command recovers after database restart',90);
    await readAsset(origins[2]);
    for(const server of servers)assert.equal(server.exitCode,null,'database outage does not terminate a Ferry process');
    console.log('PASS shared SQL outage is explicit; original identities, Runner command and S3 file recover without Ferry restart');
  }
  // Start a fresh live terminal if an earlier dependency fault closed it, so
  // the full-broker-outage assertion cannot pass on a stale closed flag.
  // Database recovery is eventual across pools, ownership and ID leases. Retry
  // a failed terminal handshake, then require actual shell output from the same
  // connection that will be tested during the full broker outage below.
  const recoveryCommand=process.platform==='win32'?`powershell -NoProfile -Command "Write-Output ('RECOVERED_' + (2+3))"\r`:'printf "RECOVERED_%s\\n" $((2+3))\r';
  await page.evaluate(({path,command})=>{
    window.terminal.onclose=null;window.terminal.close();window.terminalClosed=false;
    window.recoveryFrames='';window.recoveryAttempt=0;
    window.openRecoveryTerminal=()=>{
      window.recoveryAttempt++;window.recoveryFrames='';
      const socket=new WebSocket(path);window.terminal=socket;socket.binaryType='arraybuffer';
      socket.onopen=()=>socket.send(new TextEncoder().encode(command));
      socket.onmessage=event=>{
        const text=typeof event.data==='string'?event.data:new TextDecoder().decode(event.data);
        window.recoveryFrames+=text;
        if(text.includes('\x1b[6n'))socket.send(new TextEncoder().encode('\x1b[1;1R'));
      };
      socket.onclose=()=>{window.terminalClosed=true;};
    };window.openRecoveryTerminal();
  },{path:B.replace('http:','ws:')+'/api'+root+'/terminal-sessions',command:recoveryCommand});
  await until(()=>page.evaluate(()=>{
    if(window.terminal.readyState===WebSocket.OPEN&&window.recoveryFrames.includes('RECOVERED_5'))return true;
    if(window.terminal.readyState===WebSocket.CLOSED)window.openRecoveryTerminal();
    return false;
  }),'actual terminal shell recovers before full broker outage',90);
  console.log('PASS fresh terminal executes real shell after dependency recovery; attempts',await page.evaluate(()=>window.recoveryAttempt));
  await page.evaluate(()=>{window.terminalClosed=false;});
  for(const broker of brokers)await stop(broker);
  await until(()=>page.evaluate(()=>window.terminalClosed),'terminal closes on broker failure',35);
  for(let n=0;n<natsCount;n++)brokers[n]=await startBroker(n,'-restarted');
  if(natsCount===3)await until(replicated,'three-node broker full restart restores replicated KV stores',90);
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
  for(const name of ['runner','server-0','server-1','server-2','nats-0','nats-1','nats-2']){
    const diagnostic=await readFile(join(work,`${name}.log`),'utf8').catch(()=>'(no log)');
    console.error(`${name} diagnostics:\n${diagnostic.replace(/[a-f0-9]{32,}/gi,'[redacted]').slice(-12000)}`);
  }
  console.error('Cluster acceptance failed; isolated logs:',work);throw error;
} finally {
  if(browser)await browser.close();
  for(const socket of sockets)socket.destroy();
  if(gateway)await new Promise(done=>gateway.close(done));
  for(const child of processes.reverse())await stop(child);
  for(const log of logs)await log.close();
  if(process.env.KEEP_CLUSTER_TEST_DATA==='1')console.log('Cluster acceptance evidence:',work);
  if(!failed&&process.env.KEEP_CLUSTER_TEST_DATA!=='1'){
    assert(resolve(work).startsWith(resolve(tmpdir())+'/')||resolve(work).startsWith(resolve(tmpdir())+'\\'));
    assert(work.includes('cluster-acceptance-'));
    await rm(work,{recursive:true,force:true});
  }
}
