/** Isolated HTTP/UI regression for shared repository authentication; no user data or external writes. */
import assert from 'node:assert/strict';
import {spawn,execFileSync} from 'node:child_process';
import {once} from 'node:events';
import {createServer} from 'node:http';
import {mkdtemp,mkdir,open,readFile,writeFile} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join,resolve} from 'node:path';
import {DatabaseSync} from 'node:sqlite';
import {chromium} from 'playwright';

const output=resolve(process.env.TEST_OUTPUT||'data/maintenance/git-capabilities-20261109/connection-acceptance');
await mkdir(output,{recursive:true});
const work=await mkdtemp(join(tmpdir(),'repository-connection-reuse-'));
const report={work,checks:[],screenshots:[]};
const pass=name=>{report.checks.push(name);console.log('PASS',name);};
const wait=async(fn,timeout=45000)=>{const deadline=Date.now()+timeout;while(Date.now()<deadline){const result=await fn();if(result)return result;await new Promise(r=>setTimeout(r,150));}throw Error('Timed out');};
const requests=[];
let gitFixtureRoot;
const upstream=createServer(async(req,res)=>{
  if(req.url.startsWith('/fixture.git/')){
    if(!gitFixtureRoot){res.writeHead(404);res.end();return;}
    try{
      const url=new URL(req.url,'http://fixture'),chunks=[];for await(const chunk of req)chunks.push(chunk);
      const raw=execFileSync('git',['http-backend'],{windowsHide:true,input:Buffer.concat(chunks),env:{...process.env,GIT_PROJECT_ROOT:gitFixtureRoot,GIT_HTTP_EXPORT_ALL:'1',REQUEST_METHOD:req.method,PATH_INFO:url.pathname,QUERY_STRING:url.search.slice(1),CONTENT_TYPE:req.headers['content-type']??'',REMOTE_ADDR:'127.0.0.1'}});
      const crlf=raw.indexOf('\r\n\r\n'),split=crlf>=0?crlf:raw.indexOf('\n\n');assert(split>=0);
      for(const line of raw.subarray(0,split).toString().split(/\r?\n/)){const colon=line.indexOf(':');if(colon<0)continue;const name=line.slice(0,colon),value=line.slice(colon+1).trim();if(name.toLowerCase()==='status')res.statusCode=Number(value.split(' ')[0]);else res.setHeader(name,value);}
      res.end(raw.subarray(split+(crlf>=0?4:2)));
    }catch{res.writeHead(500);res.end();}return;
  }
  requests.push({url:req.url,token:req.headers['private-token']});
  res.setHeader('Content-Type','application/json');
  const project={id:42,name:'app',path_with_namespace:'team/app',http_url_to_repo:`${remote}/team/app.git`,default_branch:'main',web_url:`${remote}/team/app`};
  if(req.url.split('?')[0]==='/api/v4/user')res.end(JSON.stringify({id:1,username:'acceptance'}));
  else if(req.url.split('?')[0]==='/api/v4/projects')res.end(JSON.stringify([project,{...project,id:43,name:'worker',path_with_namespace:'team/sub/worker',http_url_to_repo:`${remote}/team/sub/worker.git`}]));
  else res.end(JSON.stringify(project));
});
await new Promise(r=>upstream.listen(0,'127.0.0.1',r));
const remote=`http://127.0.0.1:${upstream.address().port}`;
const reservation=createServer();await new Promise(r=>reservation.listen(0,'127.0.0.1',r));
const origin=`http://127.0.0.1:${reservation.address().port}`;await new Promise(r=>reservation.close(r));
const env={...process.env,FERRY_BIND:new URL(origin).host,FERRY_DATA_DIR:work,FERRY_SANDBOX_PATH:resolve('target/debug/ferry-sandbox.exe')};
for(const key of ['FERRY_DATABASE_URL','FERRY_NATS_URL','FERRY_CLUSTER_NAMESPACE'])delete env[key];
const log=await open(join(work,'server.log'),'a');
let server,browser,runner;
const catalog=JSON.parse(await readFile('src/shared/workflow-contracts/workflow-integration-nodes.json','utf8'));
const repositoryText=JSON.parse(await readFile('src/shared/i18n/locales/host-repository-domain.json','utf8'))['zh-CN'];
const node=(id,type,params)=>({id,name:id,type,type_version:1,params,position:[id==='start'?0:320,160]});
const graph=(type,params)=>({engine:'ferry-graph',schema:1,nodes:[node('start','core.trigger.manual',{input_schema:{type:'object'}}),node('repository',type,params)],connections:[{from:{node:'start',port:'main',index:0},to:{node:'repository',port:'main',index:0}}]});
try{
  server=spawn(resolve(process.env.FERRY_BINARY||'target/debug/ferry-server.exe'),[],{env,windowsHide:true,stdio:['ignore',log.fd,log.fd]});
  await wait(async()=>{assert.equal(server.exitCode,null);try{return(await fetch(origin+'/api/status')).ok;}catch{return false;}});
  browser=await chromium.launch({channel:'chrome',headless:true});
  const context=await browser.newContext({viewport:{width:1440,height:1000}});
  const api=context=>async(path,data,method=data?'POST':'GET',status)=>{
    const response=await context.request.fetch(origin+'/api'+path,{method,headers:{'X-Ferry-Request':'1','Accept-Language':'zh-CN'},...(data?{data}:{})});
    const text=await response.text();assert(status?response.status()===status:response.ok(),`${method} ${path}: ${response.status()} ${text}`);
    return text?JSON.parse(text):undefined;
  };
  const admin=api(context);
  await admin('/setup',{name:'Owner',email:'owner@example.invalid',password:'connection-owner-password',token:(await readFile(join(work,'setup-token'),'utf8')).trim()});
  const workspace=await admin('/workspaces',{name:'Connection acceptance',slug:'connection-acceptance'});
  const other=await admin('/workspaces',{name:'Other workspace',slug:'connection-other'});
  const root='/workspaces/'+workspace.id;
  const credential=await admin(root+'/credentials',{name:'GitLab build token',kind:'token',scope:'build',secret:'PRIVATE_ACCEPTANCE_TOKEN'});
  const connection=await admin(root+'/code-connections',{name:'Shared GitLab',provider:'gitlab',credential_id:credential.id,base_url:remote,api_url:remote+'/api/v4',organization:'',description:''});
  const params={connection_id:connection.id,project:'42'};
  const create=async(definition,name='Repository workflow',status=201)=>admin(root+'/workflows',{name,definition:JSON.stringify(definition)},'POST',status);
  const connected=await create(graph('integration.gitlab.project.get',params));
  await create(graph('integration.gitlab.project.get',{credential_id:credential.id,project:'42'}),'Denied independent',400);
  await create(graph('integration.gitlab.project.get',{connection_id:'999999',credential_id:credential.id,project:'42'}),'Missing connection',404);
  await admin('/workspaces/'+other.id+'/workflows',{name:'Cross workspace denied',definition:JSON.stringify(graph('integration.gitlab.project.get',params))},'POST',404);
  await create(graph('integration.github.repository.get',{connection_id:connection.id,owner:'team',repo:'app'}),'Wrong platform',400);
  const gitSpec=catalog.nodes.find(n=>n.git?.operation==='clone' && !n.git.provider);
  await create(graph(gitSpec.type,{...gitSpec.example_params,auth_mode:'anonymous',url:'https://example.invalid/public.git'}),'Public Git');
  await create(graph(gitSpec.type,{...gitSpec.example_params,url:'ssh://git@example.invalid/private.git'}),'Denied host authentication',400);
  await create(graph(gitSpec.type,{...gitSpec.example_params,auth_mode:'anonymous',url:'https://token@example.invalid/private.git'}),'Denied inline token',400);
  pass('HTTP save requires connections for platform/private Git; explicit public Git, missing references, platform mismatch and workspace boundaries');
  const started=await admin(root+'/workflows/'+connected.id+'/executions',{mode:'manual',revision:'draft',version:connected.version,entry_node_id:'start',inputs:{}},'POST',201);
  const finished=await wait(async()=>{const run=await admin(root+'/workflow-executions/'+started.id);return ['succeeded','failed','cancelled'].includes(run.state)?run:false;});
  assert.equal(finished.state,'succeeded',JSON.stringify(finished));
  assert(requests.some(row=>row.url==='/api/v4/projects/42'&&row.token==='PRIVATE_ACCEPTANCE_TOKEN'));
  pass('real workflow execution uses the scoped connection API origin and stored token');
  // Seed an old-format row only in this isolated fixture, then exercise the real update endpoint.
  const legacy=graph('integration.github.repository.get',{credential_id:'0',owner:'octocat',repo:'Hello-World'});
  const sql=new DatabaseSync(join(work,'ferry.sqlite'));sql.prepare('UPDATE workflows SET definition=? WHERE id=?').run(JSON.stringify(legacy),BigInt(connected.id));sql.close();
  const upgraded=await admin(root+'/workflows/'+connected.id,{version:connected.version,name:'Retained legacy authentication',definition:JSON.stringify(legacy)},'PATCH');
  const changed=structuredClone(legacy);changed.nodes[1].params.credential_id=credential.id;
  await admin(root+'/workflows/'+connected.id,{version:upgraded.version,definition:JSON.stringify(changed)},'PATCH',400);
  const copied=structuredClone(legacy);copied.nodes.push({...structuredClone(copied.nodes[1]),id:'copied'});
  await admin(root+'/workflows/'+connected.id,{version:upgraded.version,definition:JSON.stringify(copied)},'PATCH',400);
  pass('legacy auth survives ordinary saves but cannot be replaced or copied into new nodes');
  const reader=await browser.newContext();const member=api(reader);
  await member('/users',{name:'Reader',email:'reader@example.invalid',password:'connection-reader-password'});
  await member('/workspace-memberships',{code:(await admin(root+'/members/invitation')).code});
  const people=(await admin(root+'/members')).items;const person=people.find(row=>row.email==='reader@example.invalid');
  const role=await admin(root+'/roles',{name:'Workflow editor without credentials',permissions:['workflows.create','workflows.read','repositories.read']});
  await admin(root+'/members/'+person.id,{role_id:role.id,version:person.version},'PATCH');
  await member(root+'/workflows',{name:'Denied credential use',definition:JSON.stringify(graph('integration.gitlab.project.get',params))},'POST',403);
  pass('workflow editing does not bypass credentials.use');
  const page=await context.newPage();const errors=[];page.on('pageerror',error=>errors.push(error.message));
  const editable=await create(graph('integration.gitlab.project.get',params),'Shared connection editor');
  const messages=JSON.parse(await readFile('src/shared/i18n/locales/workflow-domain.json','utf8'));
  for(const [width,height] of [[1440,1000],[900,1000],[390,844],[320,740]]){
    await page.setViewportSize({width,height});
    await page.goto(origin+'/zh-CN'+root+'/workflows/'+editable.id+'/edit');
    await page.locator('.react-flow__node').filter({hasText:'repository'}).dblclick();
    const dialog=page.getByRole('dialog');
    const header=dialog.locator('header').first();
    const title=header.getByRole('heading');
    const headerBox=await header.boundingBox();
    const titleBox=await title.boundingBox();
    assert(headerBox&&titleBox);
    if(width<1024)assert(titleBox.width>=headerBox.width-40,'mobile node title retains its own full row');
    await dialog.getByLabel(messages['integration.connection'][1],{exact:true}).waitFor();
    assert.equal(await dialog.getByRole('button',{name:messages['integration.independent'][1],exact:true}).count(),0);
    await dialog.getByRole('button',{name:repositoryText['repository.020'],exact:true}).click();
    const createDialog=page.getByRole('dialog').last();
    await createDialog.getByLabel('连接名称',{exact:true}).fill('Created in editor '+width);
    await createDialog.getByLabel('凭据',{exact:true}).selectOption(credential.id);
    const advanced=createDialog.locator('details').filter({hasText:repositoryText['repository.059']});
    if(await advanced.count())await advanced.locator('summary').click();
    const base=createDialog.getByLabel(repositoryText['repository.041'],{exact:true});
    await base.fill(remote);await createDialog.getByLabel(repositoryText['repository.042'],{exact:true}).fill(remote+'/api/v4');
    const before=page.url();
    await createDialog.getByRole('button',{name:'创建',exact:true}).click();
    await wait(async()=>await page.getByRole('dialog').count()===1);
    assert.equal(page.url(),before);
    assert.equal(await page.getByRole('dialog').getByLabel(messages['integration.connection'][1],{exact:true}).locator('option:checked').textContent(),'Created in editor '+width+' · GitLab');
    const overflow=await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth+1);assert(!overflow);
    const shot=join(output,'connection-'+width+'.png');await page.screenshot({path:shot,fullPage:true});report.screenshots.push(shot);
  }
  assert.deepEqual(errors,[]);pass('Chrome Desktop/Pad/Phone: same connection picker, in-place create/autoselect, retained editor URL and no page overflow');
  const host=await admin(root+'/nodes',{name:'Checkout fixture',connection:'runner'});
  for(const [width,height] of [[1440,1000],[900,1000],[390,844],[320,740]]){
    const pipeline=await admin(root+'/pipelines',{name:'Batch checkout '+width,definition_format:'github-actions',definition:'jobs:\n  build:\n    name: Batch build\n    runs-on: self-hosted\n    steps:\n      - name: Existing command\n        run: pwd\n',executor_bindings:[{job_id:'build',target_kind:'host',target_ref:host.id,image:'host'}]});
    await page.setViewportSize({width,height});
    await page.goto(origin+'/zh-CN'+root+'/pipelines/'+pipeline.id+'/edit');
    await page.getByRole('button',{name:'Batch build',exact:true}).click();
    const batch=page.locator('details').filter({hasText:messages['actions.cloneMultiple'][1]});
    await batch.locator('summary').click();
    await batch.getByLabel('仓库连接',{exact:true}).selectOption(connection.id);
    await batch.getByRole('button',{name:messages['actions.selectGroups'][1],exact:true}).click();
    await page.getByRole('menuitemcheckbox',{name:'team · 2',exact:true}).click();
    assert.equal(await page.getByRole('menuitemcheckbox',{name:'team/sub · 1',exact:true}).getAttribute('aria-checked'),'true');
    await page.keyboard.press('Escape');
    await batch.getByRole('button',{name:messages['actions.addCheckouts'][1],exact:true}).click();
    await batch.scrollIntoViewIfNeeded();
    assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth+1));
    const shot=join(output,'batch-'+width+'.png');await page.screenshot({path:shot,fullPage:true});report.screenshots.push(shot);
    await page.getByRole('button',{name:'保存',exact:true}).click();
    await wait(async()=>{
      const saved=await admin(root+'/pipelines/'+pipeline.id);
      if(saved.checkout_bindings.length!==2)return false;
      assert.equal(new Set(saved.checkout_bindings.map(row=>row.path)).size,2);
      assert(saved.checkout_bindings.every(row=>row.connection_id===connection.id));
      return true;
    });
  }
  assert.deepEqual(errors,[]);pass('Chrome four widths: overlapping groups deduplicate to two saved checkout steps with unique directories and connection bindings');
  if(process.env.TEST_NODE_BUILD){
    gitFixtureRoot=join(work,'git-http');await mkdir(gitFixtureRoot,{recursive:true});
    const source=join(work,'source');await mkdir(source);
    const git=(...args)=>execFileSync('git',args,{windowsHide:true,stdio:'pipe'});
    git('init',source);
    await writeFile(join(source,'main.go'),'package main\nimport "fmt"\nfunc main(){fmt.Println("NODE_BUILD_OK")}\n');
    git('-C',source,'add','.');git('-C',source,'-c','user.name=Fixture','-c','user.email=fixture@example.invalid','commit','-m','Build fixture');
    git('clone','--bare',source,join(gitFixtureRoot,'fixture.git'));git('-C',join(gitFixtureRoot,'fixture.git'),'update-server-info');
    const token=await admin(root+'/runner-tokens',{host_id:host.id});
    runner=spawn(resolve('target/debug/ferry-runner.exe'),['--server',origin,'--token',token.token,'--state-dir',join(work,'agent')],{env:{...process.env,PATH:process.env.PATH+';C:\\Program Files\\Git\\bin'},windowsHide:true,stdio:['ignore',log.fd,log.fd]});
    await wait(async()=>{assert.equal(runner.exitCode,null);return(await admin(root+'/nodes/'+host.id)).state==='online';});
    const connectionFacts=await admin(root+'/nodes/'+host.id+'/connection-credentials');
    assert.equal(connectionFacts.os,process.platform==='win32'?'windows':process.platform==='darwin'?'macos':'linux');
    assert.equal(connectionFacts.installation_scope,'foreground');
    pass('Real foreground Runner handshake reports OS and installation scope without inferring a system service');
    const directory=join(work,'checkout').replaceAll('\\','/');
    const target={runtime:'host',target_kind:'host',target_ref:host.id};
    const start=node('start','core.trigger.manual',{input_schema:{type:'object'}});
    const clone={...node('clone','integration.git.repository.clone',{url:remote+'/fixture.git',directory,depth:1,auth_mode:'anonymous'}),settings:target};
    const command={...node('build','host.command',{shell:'sh',command:`cd '${directory}'\ngo build -o artifact.exe main.go\n./artifact.exe\nprintf '%s\\n' "$FERRY_INPUT_MESSAGE"`,input_env:{MESSAGE:'$(printf UNEXPECTED)'},input_assets:{}}),settings:target};
    const definition={engine:'ferry-graph',schema:1,nodes:[start,clone,command],connections:[['start','clone'],['clone','build']].map(([from,to])=>({from:{node:from,port:'main',index:0},to:{node:to,port:'main',index:0}}))};
    const flow=await create(definition,'Clone and build Go on Agent');
    const run=async(flow,inputs={})=>{
      const started=await admin(root+'/workflows/'+flow.id+'/executions',{mode:'manual',revision:'draft',version:flow.version,entry_node_id:'start',inputs},'POST',201);
      return wait(async()=>{const row=await admin(root+'/workflow-executions/'+started.id);return ['succeeded','failed','cancelled'].includes(row.state)?row:false;},150000);
    };
    const result=await run(flow);assert.equal(result.state,'succeeded',JSON.stringify(result));
    const attempts=(await admin(root+'/workflow-executions/'+result.id+'/node-runs')).items;
    const built=attempts.find(row=>row.node_id==='build');
    assert.equal(attempts.filter(row=>row.node_id==='build').length,1);
    const buildOutput=await admin(root+'/workflow-executions/'+result.id+'/node-runs/'+built.id+'/output');
    const value=buildOutput.ports[0].items[0].json;
    assert.equal(value.execution_host.id,host.id);assert.equal(value.exit_code,0);
    assert.match(value.stdout,/NODE_BUILD_OK/);assert.match(value.stdout,/\$\(printf UNEXPECTED\)/);
    assert((await readFile(join(directory,'artifact.exe'))).length>1000);
    pass('actual Agent clones HTTP Git, compiles Go, executes artifact and preserves literal input environment values');
    const bad=structuredClone(definition);bad.nodes=[start,{...command,params:{...command.params,command:'printf BUILD_EXPECTED_FAILURE >&2\nexit 7'}}];bad.connections=[{from:{node:'start',port:'main',index:0},to:{node:'build',port:'main',index:0}}];
    const failed=await run(await create(bad,'Expected build failure'));assert.equal(failed.state,'failed');
    const failures=(await admin(root+'/workflow-executions/'+failed.id+'/node-runs')).items;
    const failedData=await admin(root+'/workflow-executions/'+failed.id+'/node-runs/'+failures.find(row=>row.node_id==='build').id+'/data');
    assert.match(failedData.logs,/BUILD_EXPECTED_FAILURE/);
    pass('nonzero command fails workflow and retains stderr in node execution logs');
    const batchGraph=structuredClone(bad),marker=join(work,'batch-count').replaceAll('\\','/');
    batchGraph.nodes=[start,node('split','core.split',{path:'parts',include:'all'}),{...command,params:{...command.params,command:`printf X >> '${marker}'`}}];
    batchGraph.connections=[['start','split'],['split','build']].map(([from,to])=>({from:{node:from,port:'main',index:0},to:{node:to,port:'main',index:0}}));
    assert.equal((await run(await create(batchGraph,'Batch command once'),{parts:[1,2,3]})).state,'succeeded');
    assert.equal(await readFile(marker,'utf8'),'X');
    pass('three input items run a batch command exactly once');
    const noTerminal=await admin(root+'/roles',{name:'Run without host terminal',permissions:['workflows.create','workflows.read','workflows.run','hosts.read']});
    const freshMember=(await admin(root+'/members')).items.find(row=>row.id===person.id);
    await admin(root+'/members/'+person.id,{role_id:noTerminal.id,version:freshMember.version},'PATCH');
    const denied=await member(root+'/workflows',{name:'Host permission denied',definition:JSON.stringify(bad)},'POST',201);
    const deniedStart=await member(root+'/workflows/'+denied.id+'/executions',{mode:'manual',revision:'draft',version:denied.version,entry_node_id:'start',inputs:{}},'POST',201);
    const deniedRun=await wait(async()=>{const row=await member(root+'/workflow-executions/'+deniedStart.id);return ['succeeded','failed'].includes(row.state)?row:false;});
    assert.equal(deniedRun.state,'failed');
    const deniedNodes=(await member(root+'/workflow-executions/'+deniedStart.id+'/node-runs')).items;
    const deniedData=await member(root+'/workflow-executions/'+deniedStart.id+'/node-runs/'+deniedNodes.find(row=>row.node_id==='build').id+'/data');
    assert(!String(deniedData.logs).includes('BUILD_EXPECTED_FAILURE'));
    pass('workflows.run without hosts.terminal cannot execute a command on the host');
    const unsupported=structuredClone(bad);unsupported.nodes[1].params.shell='powershell';
    const unsupportedFlow=await create(unsupported,'Unsupported shell');
    await admin(root+'/workflows/'+unsupportedFlow.id+'/executions',{mode:'manual',revision:'draft',version:unsupportedFlow.version,entry_node_id:'start',inputs:{}},'POST',400);
    for(const [width,height] of [[1440,1000],[900,1000],[390,844],[320,740]]){
      await page.setViewportSize({width,height});await page.goto(origin+'/zh-CN'+root+'/workflows/'+flow.id+'/edit');
      await page.getByRole('button',{name:messages.add[1],exact:true}).click();
      const picker=page.getByRole('dialog');await picker.getByRole('button',{name:messages['category.build'][1],exact:true}).click();
      await picker.getByRole('button',{name:messages.hostCommand[1],exact:true}).waitFor();
      await picker.getByRole('button',{name:'Git · 仓库 · 克隆仓库',exact:true}).waitFor();
      await page.screenshot({path:join(output,'build-picker-'+width+'.png')});
      await picker.getByRole('button',{name:messages.close[1],exact:true}).click();
      await page.locator('.react-flow__node').filter({hasText:'build'}).dblclick();
      await page.getByRole('dialog').getByLabel(messages.commandScript[1],{exact:true}).scrollIntoViewIfNeeded();
      assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth+1));
      await page.screenshot({path:join(output,'build-command-'+width+'.png')});
    }
    pass('Chrome four widths exposes clone and command together and structured command editor; unsupported shell blocked before run');
  }
  if(process.env.TEST_BRANCH_UI){
    await create(graph('integration.codeup.repository.clone',{connection_id:connection.id,url:remote+'/team/app.git',directory:'/srv/app'}),'Codeup rejects GitLab connection',400);
    const database=new DatabaseSync(join(work,'ferry.sqlite'));
    const codeupId=String(BigInt(database.prepare('SELECT CAST(MAX(id) AS TEXT) AS id FROM repository_connections').get().id)+1000n);
    const columns=database.prepare('PRAGMA table_info(repository_connections)').all().map(row=>row.name);
    database.prepare('INSERT INTO repository_connections ('+columns.map(c=>'"'+c+'"').join(',')+') SELECT '+columns.map(c=>c==='id'?'?':c==='provider'?"'codeup'":c==='name'?"'Codeup UI fixture'":'"'+c+'"').join(',')+' FROM repository_connections WHERE id=?').run(BigInt(codeupId),BigInt(connection.id));database.close();
    await page.route('**/code-connections/'+codeupId+'/repositories?*',route=>route.fulfill({json:{items:[{name:'app',full_name:'team/app',url:'https://codeup.example/team/app.git',remote_id:'42'}],has_more:false}}));
    const {startBlank}=await import('./ui-template-chooser.mjs');
    for(const [width,height] of [[1440,1000],[900,1000],[390,844],[320,740]]){
      await page.setViewportSize({width,height});await page.goto(origin+'/zh-CN'+root+'/workflows/new');await startBlank(page);
      await page.getByRole('button',{name:messages.startWorkflow[1],exact:true}).click();
      let picker=page.getByRole('dialog');await picker.getByRole('button',{name:messages['category.integrations'][1],exact:true}).click();
      await picker.getByRole('button',{name:'Codeup',exact:true}).click();await picker.getByRole('button',{name:messages['integration.resource.repository'][1],exact:true}).click();
      await picker.getByRole('button',{name:'Codeup · 仓库 · 克隆仓库',exact:true}).click();
      await page.locator('.react-flow__node').dblclick();let editor=page.getByRole('dialog');
      await editor.getByLabel(messages['integration.connection'][1],{exact:true}).selectOption(codeupId);
      assert(!(await editor.getByLabel(messages['integration.connection'][1],{exact:true}).textContent()).includes('Shared GitLab'));
      await editor.getByLabel(messages['integration.chooseRepository'][1],{exact:true}).selectOption('https://codeup.example/team/app.git');
      assert.equal(await editor.locator('input[aria-label="'+messages['integration.gitUrl'][1]+'"]').count(),0);
      assert(await editor.getByText('https://codeup.example/team/app.git',{exact:true}).count());
      await page.screenshot({path:join(output,'codeup-clone-'+width+'.png')});
      const routing=graph('core.switch',{cases:[{key:'release',condition:{combinator:'and',rules:[{left:1,operator:'eq',right:1}]}},{key:'test',condition:{combinator:'and',rules:[{left:2,operator:'eq',right:2}]}}],match:'first',fallback:true});
      const flow=await create(routing,'Branch layout '+width);
      await page.goto(origin+'/zh-CN'+root+'/workflows/'+flow.id+'/edit');
      const card=page.locator('.react-flow__node[data-id="repository"]');
      await card.getByRole('button',{name:messages.addRight[1]+' · test',exact:true}).click();
      picker=page.getByRole('dialog');assert.equal(await picker.getByLabel(messages.continueFrom[1],{exact:true}).locator('option:checked').textContent(),'test');
      await picker.getByRole('button',{name:messages.no_op[1],exact:true}).click();
      await page.getByRole('button',{name:'保存',exact:true}).click();
      let saved=await wait(async()=>{const row=await admin(root+'/workflows/'+flow.id);return JSON.parse(row.definition).nodes.length===3?row:false;});
      let definition=JSON.parse(saved.definition),link=definition.connections.at(-1);assert.equal(link.from.port,'case');assert.equal(link.from.index,1);
      // Drag the actual successor below the switch; then arrange and read the persisted graph.
      await page.waitForTimeout(250);
      const target=page.locator('.react-flow__node[data-id="'+link.to.node+'"]');
      await page.locator('.react-flow__controls-fitview').click();await page.waitForTimeout(150);
      await page.screenshot({path:join(output,'branch-before-drag-'+width+'.png')});
      const fromBox=await card.boundingBox(),toBox=await target.boundingBox();assert(fromBox&&toBox);
      await page.mouse.move(toBox.x+45,toBox.y+30);await page.mouse.down();await page.mouse.move(fromBox.x+45,fromBox.y+fromBox.height+100,{steps:15});await page.mouse.up();
      await page.waitForTimeout(150);
      report.checks.push('Drag geometry '+width+': '+JSON.stringify({fromBox,toBox,after:await target.getAttribute('style')}));
      await page.screenshot({path:join(output,'branch-after-drag-'+width+'.png')});
      await page.getByRole('button',{name:messages.arrange[1],exact:true}).click();await page.getByRole('button',{name:'保存',exact:true}).click();
      saved=await wait(async()=>{const row=await admin(root+'/workflows/'+flow.id);return row.version>saved.version?row:false;});definition=JSON.parse(saved.definition);
      const sourceNode=definition.nodes.find(n=>n.id===link.from.node),targetNode=definition.nodes.find(n=>n.id===link.to.node);
      assert(targetNode.position[1]>sourceNode.position[1]&&Math.abs(targetNode.position[0]-sourceNode.position[0])<224,JSON.stringify(definition));
      assert.equal(definition.connections.at(-1).from.index,1);
      assert(await page.locator('.react-flow__edgelabel-renderer').getByText('test',{exact:true}).count());
      await page.screenshot({path:join(output,'branch-layout-'+width+'.png')});
      assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth+1));
    }
    assert.deepEqual(errors,[]);pass('Four widths: Codeup child clone uses Codeup connection and selected repository without duplicate URL input (repository discovery is a UI fixture); second branch add, real drag, arrange, save/read preserve downward direction and case index/name');
  }
  report.result='PASS';
}catch(error){report.result='FAIL';report.error=String(error.stack||error);throw error;}
finally{
  await writeFile(join(output,'result.json'),JSON.stringify(report,null,2));
  await browser?.close();
  if(runner?.exitCode===null){const stopped=once(runner,'exit');runner.kill();await stopped;}
  if(server?.exitCode===null){const stopped=once(server,'exit');server.kill();await stopped;}
  await new Promise(r=>upstream.close(r));await log.close();
}
