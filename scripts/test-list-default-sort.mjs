/** Isolated real HTTP acceptance for scenario-specific list defaults. */
import assert from 'node:assert/strict';
import {spawn} from 'node:child_process';
import {once} from 'node:events';
import {mkdtemp,open,readFile,writeFile} from 'node:fs/promises';
import {createServer} from 'node:net';
import {tmpdir} from 'node:os';
import {join,resolve} from 'node:path';
import {request} from 'playwright';

const work=await mkdtemp(join(tmpdir(),'list-default-sort-'));
const socket=createServer();await new Promise(r=>socket.listen(0,'127.0.0.1',r));
const port=socket.address().port;await new Promise(r=>socket.close(r));
const origin=`http://127.0.0.1:${port}`;
const env={...process.env,FERRY_DATA_DIR:work,FERRY_BIND:`127.0.0.1:${port}`};
for(const key of ['FERRY_DATABASE_URL','FERRY_REDIS_URL','FERRY_NATS_URL','FERRY_CLUSTER_NAMESPACE'])delete env[key];
const log=await open(join(work,'server.log'),'w');
const server=spawn(resolve(process.env.FERRY_BINARY||`target/debug/ferry-server${process.platform==='win32'?'.exe':''}`),[],{env,windowsHide:true,stdio:['ignore',log.fd,log.fd]});
const report={work,checks:[]};let context;
try{
  const deadline=Date.now()+45000;
  for(;;){assert.equal(server.exitCode,null);try{if((await fetch(origin+'/api/status')).ok)break;}catch{}assert(Date.now()<deadline,'Server ready');await new Promise(r=>setTimeout(r,100));}
  context=await request.newContext({baseURL:origin+'/api',extraHTTPHeaders:{'X-Ferry-Request':'1'}});
  const api=async(path,data,method=data?'POST':'GET')=>{
    const response=await context.fetch(origin+'/api'+path,{method,...(data?{data}:{})});
    assert(response.ok(),`${method} ${path}: ${response.status()} ${await response.text()}`);
    return response.status()===204?null:response.json();
  };
  await api('/setup',{name:'Sort acceptance',email:'sort@example.invalid',password:'temporary-sort-acceptance',token:(await readFile(join(work,'setup-token'),'utf8')).trim()});
  const workspace=await api('/workspaces',{name:'Sort acceptance',slug:'sort-acceptance'}),root=`/workspaces/${workspace.id}`;
  const graph=JSON.stringify({engine:'ferry-graph',schema:1,nodes:[],connections:[],settings:{}});
  const target=await api(root+'/nodes',{name:'Environment target',connection:'runner'});
  const fixtures=[
    ['/nodes',name=>({name,connection:'runner'})],
    ['/credentials',name=>({name,scope:'host',kind:'password',secret:'temporary-secret'})],
    ['/repositories',name=>({name,url:'https://example.invalid/source.git'})],
    ['/pipeline-groups',name=>({name})],
    ['/pipeline-environments',name=>({name,target_kind:'host',target_ref:target.id})],
    ['/config-templates',name=>({name,content:'example',parameters:'[]'})],
    ['/workflows',name=>({name,definition:graph})],
    ['/pipelines',name=>({name,definition_format:'github-actions',definition:'name: CI\non: workflow_dispatch\njobs:\n  build:\n    runs-on: self-hosted\n    steps:\n      - run: echo "$GITHUB_JOB"\n',executor_bindings:[{job_id:'build',target_kind:'host',target_ref:target.id,image:'host'}]})],
  ];
  for(const [path,body] of fixtures){
    const first=await api(root+path,body('Alpha first'));
    const second=await api(root+path,body('Zulu latest'));
    const ids=[second.id,first.id];
    const newest=await api(root+path+'?page_size=200');
    assert.deepEqual(newest.items.filter(row=>ids.includes(row.id)).map(row=>row.id),ids,path+' newest first');
    const named=await api(root+path+'?sort=name&page_size=200');
    assert.deepEqual(named.items.filter(row=>ids.includes(row.id)).map(row=>row.id),[first.id,second.id],path+' explicit name sort');
    if(path==='/workflows'||path==='/pipelines'){
      const older=await api(root+path+'/'+first.id);
      await api(root+path+'/'+first.id,{...body('Alpha edited'),version:older.version},'PATCH');
      assert.equal((await api(root+path)).items[0].id,first.id,path+' most recently edited first');
    }
    report.checks.push(path+': newest default, explicit name override'+(['/workflows','/pipelines'].includes(path)?', recently edited first':''));
  }
  const roles=[];
  for(const name of ['Zulu role','Alpha role'])roles.push(await api(root+'/roles',{name,permissions:[]}));
  const namedRoles=(await api(root+'/roles?page_size=200')).items.filter(row=>roles.some(role=>role.id===row.id));
  assert.deepEqual(namedRoles.map(row=>row.name),['Alpha role','Zulu role']);
  report.checks.push('Role reference list keeps alphabetical order');
  const first=await api(root+'/nodes?page_size=1'),second=await api(root+'/nodes?page_size=1&page=2');
  assert.notEqual(first.items[0].id,second.items[0].id);
  assert.equal(first.total,second.total);
  const other=await api('/workspaces',{name:'Other sort workspace',slug:'other-sort-workspace'});
  assert.equal((await api(`/workspaces/${other.id}/nodes`)).total,0);
  report.checks.push('Resource pagination and workspace isolation');
  report.result='PASS';await writeFile(join(work,'acceptance.json'),JSON.stringify(report,null,2));
  console.log(JSON.stringify(report,null,2));
}finally{
  await context?.dispose();
  if(server.exitCode===null&&server.signalCode===null){const ended=once(server,'exit');server.kill();await ended;}
  await log.close();
}
