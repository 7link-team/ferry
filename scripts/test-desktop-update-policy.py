#!/usr/bin/env python3
"""CI-only real signatures for privileged desktop update admission."""
import json, os, shutil, struct, subprocess, sys, tempfile, warnings, zipfile
from pathlib import Path

if os.name == 'nt':
    sys.stdout.reconfigure(encoding='utf-8', errors='replace')

with tempfile.TemporaryDirectory(prefix='desktop-update-policy-') as directory:
    root=Path(directory)
    product=json.loads(Path('desktop/tauri.conf.json').read_text())['productName']
    npm='npm.cmd' if os.name=='nt' else 'npm'
    def signer(*args):
        subprocess.run([npm,'exec','tauri','signer',*args],check=True,stdout=subprocess.DEVNULL,stderr=subprocess.PIPE)
    for name in ['key','other']:
        signer('generate','--','--ci','--password','','--write-keys',str(root/name))
    valid=f'{product}_9.9.9_windows-x86_64-update.zip'
    for name, key, version in [(valid,'key','9.9.9'),('other-product_9.9.9_windows-x86_64-update.zip','key','9.9.9'),('unversioned.exe','key',None),('wrong-key.exe','other','9.9.9')]:
        (root/name).write_bytes(b'PKsigned archive fixture\n')
        signer('sign','--','--private-key-path',str(root/key),'--password','',*(['--app-version',version] if version else []),str(root/name))
    (root/'tampered.exe').write_bytes(b'PKmodified archive fixture\n')
    shutil.copy2(root/(valid+'.sig'),root/'tampered.exe.sig')
    with zipfile.ZipFile(root/'valid.zip','w') as archive:
        archive.writestr('application.exe',b'application')
        archive.writestr('resources/worker.exe',b'worker')
    for label,name in [('traversal','../outside'),('absolute','/outside'),('drive','C:/outside'),('backslash','..\\outside'),('reserved','CON.txt'),('trailing','folder./app.exe'),('alternate-stream','app.exe:stream')]:
        with zipfile.ZipFile(root/(label+'.zip'),'w') as archive: archive.writestr(name,b'bad')
    for label,names in [('case-alias',['app.exe','APP.EXE']),('duplicate',['app.exe','app.exe'])]:
        with warnings.catch_warnings():
            warnings.simplefilter('ignore',UserWarning)
            with zipfile.ZipFile(root/(label+'.zip'),'w') as archive:
                for name in names: archive.writestr(name,b'bad')
    with zipfile.ZipFile(root/'symlink.zip','w') as archive:
        entry=zipfile.ZipInfo('link');entry.create_system=3;entry.external_attr=0o120777<<16
        archive.writestr(entry,b'../outside')
    oversized=bytearray((root/'valid.zip').read_bytes())
    central=oversized.index(b'PK\x01\x02')
    struct.pack_into('<I',oversized,central+24,2*1024*1024*1024+1)
    (root/'oversize.zip').write_bytes(oversized)
    env=dict(os.environ,DESKTOP_UPDATE_TEST_DIR=directory,DESKTOP_UPDATE_PUBLIC_KEY=(root/'key.pub').read_text().strip())
    subprocess.run(['cargo','test','--locked','-p','ferry-desktop-updater','--','--include-ignored'],env=env,check=True)
    if os.name == 'nt':
        compiled=subprocess.check_output(['cargo','test','--locked','-p','ferry-desktop-updater','--no-run','--message-format=json'],env=env,text=True)
        binaries=[item['executable'] for line in compiled.splitlines() if (item:=json.loads(line)).get('reason')=='compiler-artifact' and item.get('profile',{}).get('test') and item.get('executable')]
        assert len(binaries)==1, binaries
        # Windows PowerShell 5 must not inherit PowerShell 7's module search path.
        native_env={key:value for key,value in os.environ.items() if key.lower()!='psmodulepath'}
        native=subprocess.run(['powershell.exe','-NoProfile','-NonInteractive','-ExecutionPolicy','Bypass','-File','scripts/test-desktop-update-ipc.ps1','-TestExecutable',binaries[0]],env=native_env,creationflags=subprocess.CREATE_NO_WINDOW,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,text=True,encoding='utf-8',errors='replace',timeout=180)
        print(native.stdout,flush=True)
        native.check_returncode()
