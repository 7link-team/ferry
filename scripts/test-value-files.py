#!/usr/bin/env python3
"""Actual multipart attachment persistence, permissions and typed reference acceptance."""
import http.cookiejar,json,os,socket,subprocess,tempfile,time,urllib.request,urllib.error
from pathlib import Path

server_binary = os.environ.get("FERRY_BINARY", "target/debug/ferry-server" + (".exe" if os.name == "nt" else ""))

def client():
 return urllib.request.build_opener(urllib.request.ProxyHandler({}),urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()))
with tempfile.TemporaryDirectory(prefix="typed-attachments-") as tmp:
 with socket.socket() as sock: sock.bind(("127.0.0.1",0));port=sock.getsockname()[1]
 origin=f"http://127.0.0.1:{port}"
 env=dict(os.environ,FERRY_DATA_DIR=tmp,FERRY_BIND=f"127.0.0.1:{port}");env.pop("FERRY_DATABASE_URL",None)
 def request(c,path,body=None,method=None,status=200,content_type="application/json"):
  data=body if isinstance(body,bytes) else json.dumps(body).encode() if body is not None else None
  req=urllib.request.Request(origin+path,data=data,method=method,headers={"Content-Type":content_type,"X-Ferry-Request":"1", 'Accept-Language': 'zh-CN'})
  try: response=c.open(req,timeout=30)
  except urllib.error.HTTPError as error: response=error
  raw=response.read();assert response.code==status,(path,response.code,raw)
  return json.loads(raw) if response.headers.get("Content-Type","").startswith("application/json") else raw
 flags={"creationflags":subprocess.CREATE_NO_WINDOW} if os.name=="nt" else {}
 with open(Path(tmp)/"server.log","wb") as log:
  proc=subprocess.Popen([server_binary],env=env,stdout=log,stderr=log,**flags)
  try:
   owner=client();other=client()
   for _ in range(200):
    if proc.poll() is not None: raise AssertionError((Path(tmp)/"server.log").read_text())
    try: request(owner,"/api/status");break
    except OSError:time.sleep(.1)
   request(owner,"/api/setup",{"email":"owner@example.invalid","name":"Owner","password":"test-attachment-password","token":(Path(tmp)/"setup-token").read_text().strip()})
   request(other,"/api/users",{"email":"other@example.invalid","name":"Other","password":"test-attachment-password"})
   ws=request(owner,"/api/workspaces",{"slug":"files","name":"Files"})["id"]
   ws2=request(other,"/api/workspaces",{"slug":"other","name":"Other"})["id"]
   root=f"/api/workspaces/{ws}"
   boundary="attachment-boundary"
   content=b"<script>window.untrusted=true</script>"
   upload=(f"--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"payload.html\"\r\nContent-Type: text/html\r\n\r\n".encode()+content+f"\r\n--{boundary}--\r\n".encode())
   meta=request(owner,root+"/value-files?resource=dictionaries",upload,"POST",201,f"multipart/form-data; boundary={boundary}")
   assert meta["size"]==len(content) and meta["name"]=="payload.html"
   assert request(owner,meta["url"])==content
   req=urllib.request.Request(origin+meta["url"])
   with owner.open(req) as response:
    assert response.headers["Content-Disposition"]=="attachment"
    assert response.headers["X-Content-Type-Options"]=="nosniff"
   body={"key":"attachment-test","name":"Attachment test","category":"files","value_type":"file","value_json":json.dumps([meta]),"schema_json":json.dumps({"maxFiles":1,"maxSizeMb":1})}
   created=request(owner,root+"/dictionaries",body,"POST",201)
   got=request(owner,root+"/dictionaries/"+created["id"])
   assert json.loads(got["value_json"])[0]["id"]==meta["id"]
   forged=dict(body,key="forged",value_json=json.dumps([dict(meta,size=1)]))
   request(owner,root+"/dictionaries",forged,"POST",400)
   cross=dict(body,key="cross")
   request(other,f"/api/workspaces/{ws2}/dictionaries",cross,"POST",404)
   request(other,meta["url"],status=404)
   request(client(),meta["url"],status=401)
   request(owner,root+"/settings",dict(body,key="wrong-resource"),"POST",400)
   proc.terminate();proc.wait(timeout=30)
   proc=subprocess.Popen([server_binary],env=env,stdout=log,stderr=log,**flags)
   for _ in range(200):
    try: request(owner,"/api/status");break
    except OSError:time.sleep(.1)
   assert request(owner,meta["url"])==content
   print("PASS: multipart upload, persisted typed reference, safe download headers, forged metadata rejection, cross-resource/workspace/anonymous rejection, restart persistence")
  finally:
   if proc.poll() is None:proc.terminate();proc.wait(timeout=30)
