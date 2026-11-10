"""Black-box the shipped Runner's stdio MCP adapter, including auth and error semantics."""
import json, os, subprocess, threading, uuid
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

root=Path(__file__).resolve().parents[1]
binary=Path(os.environ.get('FERRY_RUNNER_BINARY',root/'target/debug'/('ferry-runner.exe' if os.name=='nt' else 'ferry-runner')))
token=uuid.uuid4().hex
seen=[]
class Handler(BaseHTTPRequestHandler):
    def log_message(self,*args):pass
    def do_POST(self):
        assert self.path=='/call'
        assert self.headers.get('Authorization')=='Bearer '+token
        data=json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        seen.append(data)
        if data['name']=='ferry_discover':value={'workspace_id':'1','total':21,'next_offset':20,'operations':[]}
        elif data['name']=='ferry_request':
            value={'ok':False,'status':403,'error':'permission denied'} if data['arguments']['path']=='/denied' else {'ok':True,'status':200,'data':{'total':2}}
        else:self.send_error(400);return
        payload=json.dumps(value).encode()
        self.send_response(200);self.send_header('Content-Type','application/json');self.send_header('Content-Length',str(len(payload)));self.end_headers();self.wfile.write(payload)

server=ThreadingHTTPServer(('127.0.0.1',0),Handler)
thread=threading.Thread(target=server.serve_forever,daemon=True);thread.start()
env=dict(os.environ,FERRY_PLATFORM_BRIDGE_URL=f'http://127.0.0.1:{server.server_port}/call',FERRY_PLATFORM_BRIDGE_TOKEN=token)
requests=[
    {'id':1,'method':'initialize','params':{'protocolVersion':'2025-06-18'}},
    {'method':'notifications/initialized'},
    {'id':2,'method':'tools/list'},
    {'id':3,'method':'tools/call','params':{'name':'ferry_discover','arguments':{'offset':20}}},
    {'id':4,'method':'tools/call','params':{'name':'ferry_request','arguments':{'method':'GET','path':'/pipelines?page_size=1'}}},
    {'id':5,'method':'tools/call','params':{'name':'ferry_request','arguments':{'method':'GET','path':'/denied'}}},
    {'id':6,'method':'tools/call','params':{'name':'unknown_tool','arguments':{}}},
    {'id':7,'method':'ping'},
]
flags={'creationflags':subprocess.CREATE_NO_WINDOW} if os.name=='nt' else {}
try:
    result=subprocess.run([str(binary),'platform-mcp'],env=env,input=''.join(json.dumps(dict(jsonrpc='2.0',**row))+'\n' for row in requests).encode(),stdout=subprocess.PIPE,stderr=subprocess.PIPE,timeout=25,**flags)
    assert result.returncode==0,result.stderr.decode(errors='replace')
    rows=[json.loads(line) for line in result.stdout.splitlines()]
    assert [row['id'] for row in rows]==list(range(1,8)),rows
    assert rows[0]['result']['protocolVersion']=='2025-06-18'
    assert rows[0]['result']['serverInfo']['name']=='ferry'
    tools=rows[1]['result']['tools'];assert {tool['name'] for tool in tools}=={'ferry_discover','ferry_request'}
    assert 'offset' in tools[0]['inputSchema']['properties']
    assert 'transfer' in tools[1]['inputSchema']['properties']
    assert json.loads(rows[2]['result']['content'][0]['text'])['next_offset']==20
    assert json.loads(rows[3]['result']['content'][0]['text'])['data']['total']==2
    assert rows[4]['result']['isError'] is True
    assert rows[5]['error']['code']==-32000
    assert rows[6]['result']=={}
    assert token not in result.stdout.decode()
    assert len(seen)==4
    invalid=dict(env,FERRY_PLATFORM_BRIDGE_URL='https://example.invalid/call')
    rejected=subprocess.run([str(binary),'platform-mcp'],env=invalid,input=b'',stdout=subprocess.PIPE,stderr=subprocess.PIPE,timeout=10,**flags)
    assert rejected.returncode!=0 and not rejected.stdout
    print('PASS: native stdio initialization, discovery pagination, request/result, denied operations, unknown tool, ping, capability confidentiality and external bridge rejection')
finally:
    server.shutdown();server.server_close()
