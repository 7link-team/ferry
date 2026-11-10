"""Real, isolated SeaweedFS S3 acceptance. Keeps all raw evidence and data."""
import json,os,socket,subprocess,sys,tempfile,time,urllib.request
from pathlib import Path

repo=Path(__file__).resolve().parents[1]
binary=Path(sys.argv[1]).resolve()
work=Path(tempfile.mkdtemp(prefix='workflow-assets-s3-'))
sockets=[]; ports=[]
for _ in range(10):
    s=socket.socket();s.bind(('127.0.0.1',0));sockets.append(s);ports.append(s.getsockname()[1])
master,master_grpc,volume,volume_grpc,filer,filer_grpc,s3,s3_grpc,admin,admin_grpc=ports
bucket='workflow-'+os.urandom(6).hex()
env=dict(os.environ,S3_ACCESS_KEY='acceptance-key',S3_SECRET_KEY='acceptance-secret')
args=[str(binary),'mini',f'-dir={work / "data"}','-ip=127.0.0.1','-ip.bind=127.0.0.1',
    f'-master.port={master}',f'-master.port.grpc={master_grpc}',
    f'-volume.port={volume}',f'-volume.port.grpc={volume_grpc}',
    f'-filer.port={filer}',f'-filer.port.grpc={filer_grpc}',
    f'-s3.port={s3}',f'-s3.port.grpc={s3_grpc}','-s3.port.iceberg=0','-s3.port.lance=0',
    f'-admin.port={admin}',f'-admin.port.grpc={admin_grpc}','-admin.port.legacy=0','-admin.ui=false',
    f'-bucket={bucket}','-master.volumeSizeLimitMB=32','-volume.max=2']
log=(work/'s3-server.log').open('wb')
for s in sockets:s.close()
child=None
try:
    child=subprocess.Popen(args,cwd=repo,env=env,stdout=log,stderr=log,creationflags=subprocess.CREATE_NO_WINDOW if os.name=='nt' else 0)
    deadline=time.monotonic()+60
    while time.monotonic()<deadline:
        assert child.poll() is None,('S3 process exited',work)
        try:
            # An authenticated bucket rejects unsigned requests with 403 once ready.
            urllib.request.urlopen(f'http://127.0.0.1:{s3}/{bucket}',timeout=1)
            break
        except urllib.error.HTTPError as e:
            if e.code==403:break
        except OSError:pass
        time.sleep(.2)
    else:raise AssertionError(('S3 startup timeout',work))
    test_env=dict(env,FERRY_TEST_ASSET_S3='1',FERRY_ARTIFACT_STORE='s3',
        FERRY_ARTIFACT_S3_ENDPOINT=f'http://127.0.0.1:{s3}',FERRY_ARTIFACT_S3_BUCKET=bucket,
        FERRY_ARTIFACT_S3_REGION='us-east-1',FERRY_ARTIFACT_S3_PREFIX='ferry/acceptance',AWS_ACCESS_KEY_ID='acceptance-key',AWS_SECRET_ACCESS_KEY='acceptance-secret',AWS_EC2_METADATA_DISABLED='true')
    checks=[]
    with (work/'s3-tests.log').open('wb') as output:
        for test in ['prefix_bounds_create_read_delete_and_preserves_other_businesses','content_backend_create_and_limits_are_real','content_quota_counts_candidates_and_dedup_does_not_charge_twice',
            'content_competing_uploads_reserve_quota_once','content_reference_rollback_grace_and_gc_generation',
            'content_deleting_rejects_references_and_missing_or_corrupt_data','content_pin_and_reference_race_with_gc_cannot_lose_live_data',
            'large_packets_are_encrypted_referenced_and_resume_with_original_parameters']:
            result=subprocess.run(['cargo','test','-p','ferry-server',test,'--','--nocapture','--test-threads=1'],cwd=repo,env=test_env,stdout=output,stderr=output,creationflags=subprocess.CREATE_NO_WINDOW if os.name=='nt' else 0)
            assert result.returncode==0,(test,work/'s3-tests.log')
            checks.append(test);print('PASS:',test,flush=True)
    (work/'results.json').write_text(json.dumps({'result':'PASS','backend':'SeaweedFS S3','prefix':test_env['FERRY_ARTIFACT_S3_PREFIX'],'checks':checks,'ports':ports},indent=2),encoding='utf-8')
finally:
    if child and child.poll() is None:
        child.terminate()
        try:child.wait(timeout=15)
        except subprocess.TimeoutExpired:child.kill();child.wait(timeout=15)
    log.close()
    print('S3 evidence:',work,flush=True)
    for port in ports:
        try:
            with socket.create_connection(('127.0.0.1',port),timeout=.3):
                raise AssertionError(('S3 port still listening',port))
        except OSError:pass
