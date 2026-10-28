#!/usr/bin/env python3
"""CI-only real signatures for privileged desktop update admission."""
import json, os, shutil, subprocess, tempfile
from pathlib import Path

with tempfile.TemporaryDirectory(prefix='desktop-update-policy-') as directory:
    root=Path(directory)
    product=json.loads(Path('desktop/tauri.conf.json').read_text())['productName']
    npm='npm.cmd' if os.name=='nt' else 'npm'
    def signer(*args):
        subprocess.run([npm,'exec','tauri','signer',*args],check=True,stdout=subprocess.DEVNULL,stderr=subprocess.PIPE)
    for name in ['key','other']:
        signer('generate','--','--ci','--password','','--write-keys',str(root/name))
    valid=f'{product}_9.9.9_x64-setup.exe'
    for name, key, version in [(valid,'key','9.9.9'),('other-product_9.9.9_x64-setup.exe','key','9.9.9'),('unversioned.exe','key',None),('wrong-key.exe','other','9.9.9')]:
        (root/name).write_bytes(b'MZsigned executable fixture\n')
        signer('sign','--','--private-key-path',str(root/key),'--password','',*(['--app-version',version] if version else []),str(root/name))
    (root/'tampered.exe').write_bytes(b'MZmodified executable fixture\n')
    shutil.copy2(root/(valid+'.sig'),root/'tampered.exe.sig')
    env=dict(os.environ,DESKTOP_UPDATE_TEST_DIR=directory,DESKTOP_UPDATE_PUBLIC_KEY=(root/'key.pub').read_text().strip())
    subprocess.run(['cargo','test','--locked','-p','ferry-desktop-updater','--','--include-ignored'],env=env,check=True)
