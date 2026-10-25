#!/usr/bin/env python3
"""Use throwaway keys to verify the official updater accepts valid bytes only."""
import os
from pathlib import Path
import subprocess
import tempfile

with tempfile.TemporaryDirectory(prefix='updater-signature-') as directory:
    root=Path(directory)
    (root/'payload.bin').write_bytes(b'updater verification payload\n')
    commands=[['npm','exec','tauri','signer','generate','--','--ci','--password','','--write-keys',str(root/'test.key')],
              ['npm','exec','tauri','signer','sign','--','--private-key-path',str(root/'test.key'),'--password','','--app-version','9.9.9',str(root/'payload.bin')]]
    for command in commands:
        subprocess.run(command,check=True,stdout=subprocess.DEVNULL,stderr=subprocess.PIPE)
    subprocess.run(['cargo','test','--manifest-path','desktop/Cargo.toml','--locked','signed_download_rejects','--','--ignored'],
                   env=dict(os.environ,UPDATER_TEST_DIR=directory),check=True)
