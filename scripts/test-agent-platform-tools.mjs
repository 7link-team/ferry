/** Real Server -> Runner -> native Codex -> MCP -> workspace API acceptance. */
import assert from 'node:assert/strict';
import {spawn} from 'node:child_process';
import {once} from 'node:events';
import {mkdtemp,mkdir,open,readFile,writeFile} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join,resolve} from 'node:path';
import {createServer} from 'node:net';
import {createRequire} from 'node:module';
import {dirname} from 'node:path';
import {chromium} from 'playwright';

const work=await mkdtemp(join(tmpdir(),'agent-platform-'));
const evidence=resolve(process.env.PLATFORM_TOOLS_EVIDENCE||'data/maintenance/agent-platform-tools/native');
await mkdir(evidence,{recursive:true});
const report={work,checks:[]};
const authoringOnly=process.env.PLATFORM_TOOLS_SCENARIO==='authoring';
const pass=name=>{report.checks.push(name);console.log('PASS',name);};
const listener=createServer();await new Promise(r=>listener.listen(0,'127.0.0.1',r));
const origin=`http://127.0.0.1:${listener.address().port}`;await new Promise(r=>listener.close(r));
const env={...process.env,FERRY_DATA_DIR:join(work,'server'),FERRY_BIND:origin.slice(7)};
if(process.platform==='win32') {
  const require=createRequire(join(process.env.APPDATA,'npm/node_modules/@openai/codex/package.json'));
  const vendor=join(dirname(require.resolve('@openai/codex-win32-x64/package.json')),'vendor/x86_64-pc-windows-msvc/bin');
  env.PATH=vendor+';'+env.PATH;
}
for(const key of ['FERRY_DATABASE_URL','FERRY_NATS_URL','FERRY_CLUSTER_NAMESPACE'])delete env[key];
await mkdir(env.FERRY_DATA_DIR);
const serverLog=await open(join(work,'server.log'),'a'),runnerLog=await open(join(work,'runner.log'),'a');
const suffix=process.platform==='win32'?'.exe':'';
let server,runner,browser;
async function wait(fn,seconds=240){const end=Date.now()+seconds*1000;while(Date.now()<end){const result=await fn();if(result)return result;await new Promise(r=>setTimeout(r,250));}throw Error('Acceptance timed out');}
async function stop(child){if(!child||child.exitCode!==null||child.signalCode!==null)return;const end=once(child,'exit');child.kill('SIGTERM');await end;}
try {
  server=spawn(resolve(process.env.FERRY_BINARY||`target/debug/ferry-server${suffix}`),[],{env,windowsHide:true,stdio:['ignore',serverLog.fd,serverLog.fd]});
  await wait(async()=>{assert.equal(server.exitCode,null);try{return(await fetch(origin+'/api/status')).ok;}catch{return false;}},30);
  browser=await chromium.launch({channel:'chrome',headless:true});
  const owner=await browser.newContext(),outsider=await browser.newContext();
  const api=context=>async(path,data,method=data?'POST':'GET',status)=>{
    const response=await context.request.fetch(origin+'/api'+path,{method,headers:{'X-Ferry-Request':'1','Accept-Language':'en-US'},...(data?{data}:{})});
    assert(status?response.status()===status:response.ok(),`${method} ${path}: ${response.status()} ${await response.text()}`);
    return response.status()===204?null:response.json();
  };
  const admin=api(owner),otherUser=api(outsider);
  await admin('/setup',{name:'Platform tools owner',email:'owner@example.invalid',password:'temporary-platform-tools',token:(await readFile(join(env.FERRY_DATA_DIR,'setup-token'),'utf8')).trim()});
  const workspace=await admin('/workspaces',{name:'Platform tools acceptance',slug:'platform-tools'}),root=`/workspaces/${workspace.id}`;
  const other=await admin('/workspaces',{name:'Other workspace',slug:'other'});
  await otherUser('/users',{name:'Other account',email:'other@example.invalid',password:'temporary-other-account'});
  await otherUser(root+'/nodes',undefined,'GET',404);
  const host=await admin(root+'/nodes',{name:'Real native Runner',connection:'runner',runner_auto_update:'off'});
  const enrollment=await admin(root+'/runner-tokens',{host_id:host.id});
  runner=spawn(resolve(process.env.FERRY_RUNNER_BINARY||`target/debug/ferry-runner${suffix}`),['--server',origin,'--token',enrollment.token,'--state-dir',join(work,'runner')],{env,windowsHide:true,stdio:['ignore',runnerLog.fd,runnerLog.fd]});
  await wait(async()=>{assert.equal(runner.exitCode,null);return(await admin(root+'/nodes/'+host.id)).state==='online';},30);
  const grants=['hosts.read','hosts.terminal','hosts.files','repositories.read','pipelines.read','workflows.read','workflows.create','workflows.update','workflows.run','cron-jobs.read','cron-jobs.create','cron-jobs.run','deployments.read','app-templates.read','host-groups.read','host-groups.create'];
  const role=await admin(root+'/roles',{name:'Platform tool operator',permissions:grants});
  const human=(await admin(root+'/members')).items.find(row=>row.email==='owner@example.invalid');
  const cwd=join(work,'agent');await mkdir(cwd);
  const profile=await admin(root+'/ai-models',{name:'Native CLI acceptance',kind:'cli',runtime:'codex',models:[],default_model:'',state:'enabled'});
  const agent=await admin(root+'/ai-agents',{name:'Platform access assistant',model_profile_id:profile.id,platform_role_id:role.id,owner_id:human.user_id,host_id:host.id,runtime:'codex',model:'',cwd,permission:'read_only',visibility:'workspace',state:'enabled',system_prompt:'Use the ferry_discover and ferry_request tools for platform questions. Query actual APIs. Never infer platform records from local files. If denied, report the denied operation, never invent an empty result. The server enforces the human-configured approval policy.'});
  for(let i=0;i<2;i++)await admin(root+'/workflows',{name:`Count fixture ${i+1}`,definition:JSON.stringify({engine:'ferry-graph',schema:1,nodes:[],connections:[],settings:{}})});
  await admin(root+'/pipelines',{name:'Count fixture build',definition_format:'github-actions',definition:'name: Acceptance\non: workflow_dispatch\njobs:\n  build:\n    runs-on: self-hosted\n    steps:\n      - run: echo "$GITHUB_JOB"\n',executor_bindings:[{job_id:'build',target_kind:'host',target_ref:host.id,image:'host'}]});
  const connection=await admin(root+'/code-connections',{name:'Repository fixture',provider:'git',url:'https://example.invalid/source.git'});
  report.expected={pipelines:(await admin(root+'/pipelines')).total,workflows:(await admin(root+'/workflows')).total,nodes:(await admin(root+'/nodes')).total,repositories:(await admin(root+'/code-connections')).total,deployments:(await admin(root+'/deployments')).total};
  const created=await admin(root+'/conversations',{agent_id:agent.id,title:'Actual platform tool acceptance'}),path=root+'/conversations/'+created.id;
  await otherUser(path,undefined,'GET',404);
  await admin(`/workspaces/${other.id}/ai-agents/${agent.id}`,undefined,'GET',404);
  async function events(){let after=0,rows=[];for(;;){const result=await admin(path+'/events?after='+after);rows.push(...result.items);if(!result.has_more)return rows;after=result.next;}}
  async function send(prompt){return admin(path+'/turns',{prompt,attachments:[],client_id:crypto.randomUUID()});}
  async function complete(turn){return wait(async()=>{const detail=await admin(path);const row=detail.turns.find(t=>t.id===turn.id);if(!row||['queued','running','stopping'].includes(row.state))return false;assert.equal(row.state,'completed',JSON.stringify(row));return row;});}
  let history=[];
  if(!authoringOnly){
  const first=await send('First use ferry_discover to inspect available platform API operations. Then use ferry_request with method GET for each of /pipelines?page_size=1, /workflows?page_size=1, /nodes?page_size=1, /code-connections?page_size=1 and /deployments?page_size=1. Report the total field for each of those five resources. Do not inspect local files.');
  await complete(first);
  history=await events();
  const results=history.filter(row=>row.turn_id===first.id&&row.event.type==='platform_result').map(row=>row.event.result);
  assert(results.length>=6,JSON.stringify(results));
  assert(results.some(row=>row.data?.total===2),'real workflow count must include both rows, not one page');
  assert(results.filter(row=>row.status===200).length>=5,JSON.stringify(results));
  assert(results.every(row=>row.ok!==false),JSON.stringify(results));
  pass('real Codex discovers tools and queries actual totals for five business modules');
  const questionTurn=await send('Use ferry_ask_user now with title "Acceptance choice", question "Which approach should I use?", and one optional suggestion {"id":"suggested","label":"Suggested approach"}. Do not use any other tools, do not choose for me, and end this turn after the question is saved. After my answer, repeat my answer exactly without tools.');
  await complete(questionTurn);
  const questionEntry=Object.entries((await admin(path)).turns.find(t=>t.id===questionTurn.id).decisions).find(([,d])=>d.tool==='ferry_ask_user');
  assert(questionEntry,'real native CLI must call ferry_ask_user');
  const questionPage=await owner.newPage();
  const questionErrors=[];questionPage.on('pageerror',error=>questionErrors.push(error.message));
  for(const width of [1440,900,767,390,320]){
    await questionPage.setViewportSize({width,height:900});
    await questionPage.goto(`${origin}/en-US${path}`);
    const card=questionPage.locator(`[data-user-question="${questionEntry[0]}"]`);
    await card.waitFor();
    const banner=questionPage.locator('[data-pending-banner]');
    assert((await banner.innerText()).includes('waiting for your response'));
    await banner.getByRole('button').click();
    await card.scrollIntoViewIfNeeded();
    const trigger=questionPage.locator('[data-execution-details]').last();
    await trigger.scrollIntoViewIfNeeded();
    const before=await questionPage.locator('.conversation-feed').evaluate(node=>node.scrollTop);
    await trigger.click();
    const panel=questionPage.locator('[data-execution-panel]');
    await panel.waitFor();
    const bounds=await panel.boundingBox();
    assert(bounds&&bounds.x>=0&&bounds.x+bounds.width<=width+1,`panel fits at ${width}`);
    if(width<768)assert(Math.abs(bounds.width-width)<2,'phone details must occupy the full width');
    await questionPage.screenshot({path:join(evidence,`question-panel-${width}.png`),fullPage:false});
    await questionPage.keyboard.press('Escape');
    await panel.waitFor({state:'detached'});
    await wait(()=>questionPage.evaluate(()=>document.activeElement?.matches('[data-execution-details]')),5);
    const after=await questionPage.locator('.conversation-feed').evaluate(node=>node.scrollTop);
    assert(Math.abs(before-after)<2,`closing panel preserves chat position at ${width}`);
    assert(await questionPage.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),`question page overflow at ${width}`);
  }
  const answer='Use my custom approach '+crypto.randomUUID();
  const response=await admin(`${path}/turns/${questionTurn.id}/question-answers`,{request_id:questionEntry[0],action:'continue',answer});
  await complete(response);
  const continued=await admin(path);
  assert(continued.messages.some(message=>message.turn_id===response.id&&message.role==='assistant'&&message.prompt.includes(answer)),JSON.stringify(continued.turns));
  await questionPage.reload();
  await questionPage.getByText(answer,{exact:true}).first().waitFor();
  assert.deepEqual(questionErrors,[]);
  await questionPage.close();
  pass('real CLI question survives turn completion; free-text response continues once; detail panel fits five widths and restores focus/scroll');
  }
  const current=await admin(path);
  await admin(path+'/approval-mode',{mode:'yolo',version:current.version},'PUT');
  const authoringName='Native authoring '+crypto.randomUUID();
  const authoring=await send(`Use actual platform tools to create and verify a workflow named "${authoringName}". First GET /workflow-definition-format and inspect the core.trigger.manual and data.set node contracts by exact type. Create a valid two-node draft (manual trigger then data.set with a result field containing "authoring-ok"). Use the documented graph structure and JSON-string definition. Run the draft once and inspect its execution and node outputs. Then publish that draft using current version and draft revision. Create one DISABLED scheduled job named "${authoringName}" with schedule "0 9 * * 1", timezone "UTC", timeout_seconds 600, action/target_kind "workflow", inputs {}, and the real manual entry node id. POST its /runs endpoint exactly once to test it now; GET the returned /cron-runs/{id}, follow downstream_kind/downstream_id to the actual workflow execution, and inspect it before reporting success. Do not enable this schedule, send notifications, delete records, or operate on other fixtures. The human has explicitly selected YOLO for this isolated acceptance conversation; API permissions and validation still apply.`);
  await complete(authoring);
  const authored=(await admin(root+'/workflows')).items.filter(row=>row.name===authoringName);
  assert.equal(authored.length,1,'native CLI must create exactly one workflow');
  const authoredDetail=await admin(root+'/workflows/'+authored[0].id);
  assert(authoredDetail.published_revision_id&&authoredDetail.published_revision_id!=='0','workflow must be published');
  const schedules=(await admin(root+'/cron-jobs?scope=build')).items.filter(row=>row.name===authoringName);
  assert.equal(schedules.length,1,'native CLI must create exactly one schedule');
  assert.equal(schedules[0].enabled,false,'acceptance schedule must remain disabled');
  const scheduledRuns=(await admin(root+'/cron-jobs/'+schedules[0].id+'/runs')).items;
  assert.equal(scheduledRuns.length,1,'native CLI must test the schedule exactly once');
  const scheduledRun=await wait(async()=>{const run=await admin(root+'/cron-runs/'+scheduledRuns[0].id);assert(!['failed','skipped','cancelled'].includes(run.state),JSON.stringify(run));return run.state==='succeeded'?run:false;});
  assert.equal(scheduledRun.downstream_kind,'workflow-executions');
  const execution=await admin(root+'/workflow-executions/'+scheduledRun.downstream_id);
  assert.equal(execution.state,'succeeded');
  const authoringHistory=(await events()).filter(row=>row.turn_id===authoring.id&&row.event.type==='platform_result');
  assert(authoringHistory.some(row=>row.event.call?.path==='/workflow-definition-format'&&row.event.result.ok));
  assert(!authoringHistory.some(row=>row.event.result.status===413),'authoring directory discovery must not exceed the tool response limit');
  const afterAuthoring=await admin(path);
  await admin(path+'/approval-mode',{mode:'request',version:afterAuthoring.version},'PUT');
  pass('real Codex discovers schemas, creates/runs/publishes a workflow, schedules it disabled, and follows the cron downstream execution');
  if(!authoringOnly){
  const write=await send('Use ferry_request to POST /node-groups with body {"name":"MCP approved group","parent_id":"0","position":0}. This is an explicitly requested write. Wait for the platform confirmation and report its result. Do not run local commands.');
  const pending=await wait(async()=>{const turn=(await admin(path)).turns.find(t=>t.id===write.id);return Object.entries(turn?.decisions||{}).find(([,d])=>d.tool==='ferry_request'&&d.decision===null);});
  assert(!(await admin(root+'/node-groups')).items.some(row=>row.name==='MCP approved group'));
  await admin(path+'/turns/'+write.id+'/decisions',{request_id:pending[0],allow:true});
  await complete(write);
  assert.equal((await admin(root+'/node-groups')).items.filter(row=>row.name==='MCP approved group').length,1);
  pass('real MCP write waits for confirmation and creates exactly one business record');
  const file=join(work,'mcp-transfer.txt');
  const filePath=`/nodes/${host.id}/files/content?path=${encodeURIComponent(file)}`;
  const io=await send(`Use ferry_request for these operations in order, waiting for each platform confirmation: POST /nodes/${host.id}/commands with body ${JSON.stringify({command:'echo mcp-command-ok',timeout_seconds:10})}; PUT ${filePath} with transfer ${JSON.stringify({data_base64:Buffer.from('mcp-transfer-ok\n').toString('base64')})}; GET ${filePath}. Do not use local filesystem tools. Report the actual results.`);
  const approved=new Set();
  await wait(async()=>{
    const turn=(await admin(path)).turns.find(t=>t.id===io.id);
    for(const [id,d] of Object.entries(turn?.decisions||{})) {
      if(d.tool==='ferry_request'&&d.decision===null&&!approved.has(id)) {
        await admin(path+'/turns/'+io.id+'/decisions',{request_id:id,allow:true});approved.add(id);
      }
    }
    if(!turn||['queued','running','stopping'].includes(turn.state))return false;
    assert.equal(turn.state,'completed',JSON.stringify(turn));return true;
  });
  assert.equal(await readFile(file,'utf8'),'mcp-transfer-ok\n');
  history=await events();
  const ioResults=history.filter(row=>row.turn_id===io.id&&row.event.type==='platform_result').map(row=>row.event.result);
  assert(ioResults.some(row=>row.data?.stdout?.includes('mcp-command-ok')),JSON.stringify(ioResults));
  assert(ioResults.some(row=>row.data?.data_base64===Buffer.from('mcp-transfer-ok\n').toString('base64')),JSON.stringify(ioResults));
  pass('real MCP confirmed host command and binary file upload/download');
  const savedRole=await admin(root+'/roles/'+role.id);
  await admin(root+'/roles/'+role.id,{name:savedRole.name,permissions:grants.filter(g=>g!=='pipelines.read'),version:savedRole.version},'PATCH');
  const denied=await send('Call ferry_request with method GET and path /pipelines?page_size=1 now. Do not rely on earlier results or the discovery directory. If the server denies this call, report permission denied. Do not inspect local files.');
  await complete(denied);
  history=await events();
  assert(history.some(row=>row.turn_id===denied.id&&row.event.type==='platform_result'&&row.event.result.status===403));
  pass('revoking the Agent role takes effect in the same native conversation');
  const uiErrors=[],page=await owner.newPage();page.on('pageerror',e=>uiErrors.push(e.message));
  const text=JSON.parse(await readFile('src/shared/i18n/locales/agents.json','utf8'));
  for(const [index,locale] of ['en-US','zh-CN','zh-TW','ja-JP','ko-KR','ms-MY'].entries()){
    await page.goto(`${origin}/${locale}${root}/ai-agents/${agent.id}/edit`);
    const picker=page.getByRole('combobox',{name:text.platformRole[index],exact:true});await picker.waitFor();
    await picker.click();await page.getByRole('option',{name:'Platform tool operator',exact:true}).waitFor();await page.keyboard.press('Escape');
  }
  for(const width of [1440,900,390,320]){
    await page.setViewportSize({width,height:900});await page.goto(`${origin}/en-US${root}/ai-agents/${agent.id}/edit`);
    await page.getByRole('combobox',{name:text.platformRole[0],exact:true}).waitFor();
    assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),`page overflow at ${width}`);
    await page.screenshot({path:join(evidence,`agent-role-${width}.png`),fullPage:true});
  }
  await page.setViewportSize({width:1440,height:1000});await page.goto(`${origin}/en-US${path}`);await page.locator('#conversation-message').waitFor();
  await page.screenshot({path:join(evidence,'conversation-tools.png'),fullPage:true});
  assert.deepEqual(uiErrors,[]);pass('role picker six locales and four viewport widths; tool history is visible');
  }
  history=await events();
  report.history=history;report.fixture={workspace:workspace.id,agent:agent.id,conversation:created.id,connection:connection?.id};
  report.result='PASS';
} catch(error){report.result='FAIL';report.error=error.stack;throw error;}
finally{await writeFile(join(evidence,'result.json'),JSON.stringify(report,null,2));await browser?.close();await stop(runner);await stop(server);await runnerLog.close();await serverLog.close();}
