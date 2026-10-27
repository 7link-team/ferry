#!/usr/bin/env python3
"""Real API/runner acceptance for pipeline classification and environment snapshots."""
import http.cookiejar,json,os,socket,subprocess,tempfile,time,urllib.request,urllib.error
from pathlib import Path

def main():
    work=Path(tempfile.mkdtemp(prefix='pipeline-classification-'))
    with socket.socket() as listener:
        listener.bind(('127.0.0.1',0));port=listener.getsockname()[1]
    origin=f'http://127.0.0.1:{port}'
    env=dict(os.environ,FERRY_DATA_DIR=str(work),FERRY_BIND=f'127.0.0.1:{port}')
    env.pop('FERRY_DATABASE_URL',None)
    if os.environ.get('FERRY_TEST_DATABASE_URL'):env['FERRY_DATABASE_URL']=os.environ['FERRY_TEST_DATABASE_URL']
    children=[];files=[];checks=0
    def launch(args,filename):
        output=open(work/filename,'wb');files.append(output)
        child=subprocess.Popen(args,env=env,stdout=output,stderr=output);children.append(child);return child
    client=urllib.request.build_opener(urllib.request.ProxyHandler({}),urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()))
    def api(path,body=None,method=None,status=200):
        request=urllib.request.Request(origin+'/api'+path,data=json.dumps(body).encode() if body is not None else None,method=method or ('POST' if body is not None else 'GET'),headers={'Content-Type':'application/json','X-Ferry-Request':'1'})
        try:
            with client.open(request,timeout=15) as response:code,raw=response.status,response.read()
        except urllib.error.HTTPError as error:code,raw=error.code,error.read()
        assert code==status,(path,code,raw[:500])
        return json.loads(raw) if raw else None
    def eventually(fn):
        for _ in range(300):
            value=fn()
            if value:return value
            time.sleep(.1)
        raise AssertionError('timed out')
    def check(value,label):
        nonlocal checks
        assert value,label;checks+=1;print('PASS',label,flush=True)
    root='/workspaces/classify'
    def create(resource,body,base=root):return api(base+'/'+resource,body,status=201)['id']
    def detail(resource,id):return api(root+'/'+resource+'/'+id)
    def trigger(id,environment=None,status=201):return api(root+'/pipelines/'+id+'/actions/run',{} if environment is None else {'environment_id':environment},status=status)
    def done(id):
        def finished():
            row=detail('pipeline-runs',id)
            return row if row['state'] not in ('queued','running','cancelling') else None
        return eventually(finished)
    try:
        server=launch([os.environ.get('FERRY_BINARY','target/debug/ferry-server')],'server.log')
        def ready():
            assert server.poll() is None
            try:return api('/status')['ok']
            except OSError:return False
        eventually(ready)
        api('/setup',{'token':(work/'setup-token').read_text().strip(),'email':'classify@example.invalid','name':'Admin','password':'temporary-test-password'})
        api('/workspaces',{'name':'Classify','slug':'classify'})
        api('/workspaces',{'name':'Other','slug':'other'})
        group=create('pipeline-groups',{'name':'Backend'})
        host_ids=[]
        for index in range(2):
            host=create('hosts',{'name':f'Runner {index}','connection':'runner'});host_ids.append(host)
            token=api(root+'/runner-tokens',{'host_id':host},status=201)['token']
            launch(['target/debug/ferry-runner','--server',origin,'--token',token,'--state-dir',str(work/f'runner-{index}')],f'runner-{index}.log')
            eventually(lambda:detail('hosts',host)['state']=='online')
        environments=[create('pipeline-environments',{'name':name,'target_kind':'host','target_ref':host}) for name,host in zip(['Staging','Production'],host_ids)]
        tag=create('tags',{'name':'API','color':''})
        manifest={'apiVersion':'ferry/v1','kind':'Application','metadata':{'name':'web'},'services':[{'name':'web','image':'busybox:1.36'}]}
        app=create('deployments',{'name':'Existing app','mode':'compose','target_kind':'host','target_ref':host_ids[0],'workdir':'/tmp/classify-app','manifest':json.dumps(manifest)})
        variable_ids=[]
        for environment,value in zip(environments,['staging-value','production-value']):
            variable_ids.append(create('variables',{'name':'VALUE','scope':'environment','scope_ref':environment,'kind':'plain','value':value}))
        for environment in environments:
            create('variables',{'name':'MULTILINE','scope':'environment','scope_ref':environment,'kind':'secret','value':'first-secret-marker-928\nsecond-secret-marker-174'})
        workflow="name: Classified\njobs:\n  build:\n    runs-on: self-hosted\n    steps:\n      - name: Verify environment\n        env:\n          TOKEN: ${{ secrets.MULTILINE }}\n        run: |\n          sleep 1\n          echo \"$TOKEN\"\n          case '${{ vars.VALUE }}' in staging-value) echo STAGING_OK ;; production-value) echo PRODUCTION_OK ;; *) exit 9 ;; esac\n"
        workflow += '  middle:\n    needs: build\n    runs-on: self-hosted\n    steps:\n      - run: echo MIDDLE_OK\n  final:\n    needs: middle\n    runs-on: self-hosted\n    steps:\n      - run: echo FINAL_OK\n'
        body={'name':'Shared pipeline','definition_format':'github-actions','definition':workflow,'target_kind':'host','target_ref':host_ids[0],'group_id':group,'application_id':app,'tag_ids':[tag],'environment_ids':environments}
        pipeline=create('pipelines',body)
        saved=detail('pipelines',pipeline)
        check(saved['group_id']==group and saved['application_id']==app and saved['tag_ids']==[tag] and set(saved['environment_ids'])==set(environments),'classification associations persist')
        for key,value in [('group_id',group),('application_id',app),('tag_id',tag),('environment_id',environments[0])]:
            check(api(root+'/pipelines?'+key+'='+value)['total']==1,'filter '+key)
        trigger(pipeline,status=400)
        check(True,'bound pipeline requires explicit environment')
        foreign=create('pipeline-environments',{'name':'Foreign','target_kind':'host','target_ref':create('hosts',{'name':'Foreign','connection':'runner'},'/workspaces/other')},'/workspaces/other')
        trigger(pipeline,foreign,status=400)
        create_body=dict(body,name='Foreign binding',environment_ids=[foreign])
        api(root+'/pipelines',create_body,status=400)
        check(True,'foreign environment rejected at save and trigger')
        for environment,host,value in zip(environments,host_ids,['staging-value','production-value']):
            run=done(trigger(pipeline,environment)['id'])
            check(run['state']=='succeeded' and run['host_id']==host and run['environment_id']==environment,'selected environment executes on its own host')
            chunks=''.join(row['chunk'] for row in api(root+'/pipeline-runs/'+run['id']+'/logs')['items'])
            check(('[build] STAGING_OK\n' if value=='staging-value' else '[build] PRODUCTION_OK\n') in chunks and run['variables'][0]['scope']=='environment','selected environment variables frozen in run')
            check('[final] FINAL_OK\n' in chunks,'three-level dependency chain executes its final job')
            check('first-secret-marker-928' not in chunks and 'second-secret-marker-174' not in chunks,'multiline environment secret lines stay out of persisted logs')
        snapshot_run=trigger(pipeline,environments[0])['id']
        api(root+'/variables/'+variable_ids[0],{'name':'VALUE','version':1,'value':'changed-after-trigger'},method='PATCH')
        current_environment=detail('pipeline-environments',environments[0])
        api(root+'/pipeline-environments/'+environments[0],dict(current_environment,name='Renamed stage',target_ref=host_ids[1]),method='PATCH')
        frozen=done(snapshot_run)
        check(frozen['state']=='succeeded' and frozen['host_id']==host_ids[0] and frozen['environment_name']=='Staging','trigger freezes environment name, host and variable value')
        api(root+'/variables/'+variable_ids[0],method='DELETE',status=409)
        check(True,'selected environment variable in use cannot be deleted')
        api(root+'/hosts/'+host_ids[1],method='DELETE',status=409)
        check(True,'environment default host is protected from deletion')
        cron_body={'name':'Scheduled environment','scope':'build','schedule':'0 0 1 1 *','timezone':'UTC','enabled':False,'action':'pipeline','target_kind':'pipeline','target_ref':pipeline,'timeout_seconds':30,'environment_id':environments[1]}
        cron=create('cron-jobs',cron_body)
        api(root+'/cron-jobs',dict(cron_body,name='Missing environment',environment_id=''),status=400)
        check(detail('cron-jobs',cron)['environment_id']==environments[1],'scheduled pipeline requires and preserves its chosen environment')
        api(root+'/pipelines/'+pipeline,dict(body,version=saved['version'],environment_ids=[environments[0]]),method='PATCH',status=409)
        after_rejected_unbind=detail('pipelines',pipeline)
        check(after_rejected_unbind['version']==saved['version'] and set(after_rejected_unbind['environment_ids'])==set(environments),'scheduled environment cannot be unbound and rejection is atomic')
        api(root+'/cron-jobs/'+cron,method='DELETE',status=204)
        for resource,id in [('pipeline-groups',group),('pipeline-environments',environments[0]),('tags',tag),('deployments',app)]:
            api(root+'/'+resource+'/'+id,method='DELETE',status=409)
            check(True,'referenced '+resource+' cannot be deleted')
        ordinary={key:value for key,value in body.items() if key not in ('group_id','application_id','tag_ids','environment_ids')}
        api(root+'/pipelines/'+pipeline,dict(ordinary,version=saved['version']),method='PATCH')
        preserved=detail('pipelines',pipeline)
        check(preserved['group_id']==group and preserved['tag_ids']==[tag] and set(preserved['environment_ids'])==set(environments),'omitted classification fields preserve existing associations')
        api(root+'/pipelines/'+pipeline,dict(body,version=saved['version'],tag_ids=[]),method='PATCH',status=409)
        check(detail('pipelines',pipeline)['tag_ids']==[tag],'stale update cannot clear tags')
        api(root+'/pipelines/'+pipeline,dict(ordinary,version=preserved['version'],group_id='',application_id='',tag_ids=[],environment_ids=[],definition='jobs:\n  build:\n    runs-on: self-hosted\n    steps:\n      - run: echo LEGACY_DEFAULT\n'),method='PATCH')
        cleared=detail('pipelines',pipeline)
        check(cleared['tag_ids']==[] and cleared['environment_ids']==[] and cleared['group_id']=='0','explicit empty fields clear associations')
        check(done(trigger(pipeline)['id'])['state']=='succeeded','unbound pipeline still runs with original default executor')
        default_cron=create('cron-jobs',dict(cron_body,name='Default schedule',environment_id=''))
        api(root+'/pipelines/'+pipeline,dict(ordinary,version=cleared['version'],environment_ids=[environments[0]]),method='PATCH',status=409)
        check(detail('pipelines',pipeline)['environment_ids']==[],'default schedule cannot be invalidated by adding required environments')
        api(root+'/cron-jobs/'+default_cron,method='DELETE',status=204)
        print(checks,'checks passed',flush=True)
    except Exception:
        print('diagnostics:',work,flush=True);raise
    finally:
        for child in reversed(children):
            if child.poll() is None:
                child.terminate()
                try:child.wait(timeout=90)
                except subprocess.TimeoutExpired:child.kill();child.wait()
        for output in files:output.close()
if __name__=='__main__':main()
