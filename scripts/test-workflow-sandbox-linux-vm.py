#!/usr/bin/env python3
"""Boot an ephemeral Linux kernel to run the original sandbox acceptance suite.

Usage: python scripts/test-workflow-sandbox-linux-vm.py QEMU KERNEL INITRAMFS ELF
No network adapter, host disk, directory share or privileged VM installation.
Guest requests/results are exported as a compressed archive in the serial log.
The throughput assertion is unchanged; CPU emulation is recorded explicitly.
"""
import base64
import gzip
import hashlib
import io
import json
from pathlib import Path
import re
import stat
import subprocess
import sys
import tarfile
import tempfile
import time
import urllib.request


REPO = Path(__file__).resolve().parents[1]
TOOLS = REPO / 'data/maintenance/linux-vm-tools'
MIRROR = 'https://dl-cdn.alpinelinux.org/alpine/v3.22/main/x86_64/'


def download(name):
    cache = TOOLS / 'packages' / name
    cache.parent.mkdir(parents=True, exist_ok=True)
    if not cache.exists():
        with urllib.request.urlopen(MIRROR + name, timeout=60) as response:
            data = response.read()
        cache.write_bytes(data)
    return cache.read_bytes()


def read_cpio(data):
    entries = {}
    offset = 0
    while offset + 110 <= len(data):
        assert data[offset:offset + 6] in (b'070701', b'070702')
        fields = [int(data[offset + 6 + i*8:offset + 14 + i*8], 16) for i in range(13)]
        offset += 110
        name = data[offset:offset + fields[11] - 1].decode()
        offset = (offset + fields[11] + 3) & ~3
        content = data[offset:offset + fields[6]]
        offset = (offset + fields[6] + 3) & ~3
        if name == 'TRAILER!!!':
            break
        entries[name.removeprefix('./')] = (fields[1], content, fields[9], fields[10])
    return entries


def write_cpio(entries):
    output = bytearray()
    for inode, (name, (mode, content, major, minor)) in enumerate(
            [*entries.items(), ('TRAILER!!!', (0, b'', 0, 0))], 1):
        encoded = name.encode() + b'\0'
        fields = [inode, mode, 0, 0, 1, 0, len(content), 0, 0, major, minor, len(encoded), 0]
        output.extend(b'070701' + ''.join(f'{field:08x}' for field in fields).encode())
        output.extend(encoded)
        output.extend(b'\0' * (-len(output) % 4))
        output.extend(content)
        output.extend(b'\0' * (-len(output) % 4))
    return gzip.compress(output, mtime=0)


def alpine_packages(roots=('python3',)):
    # Resolve the distro's actual shared-library dependencies; do not rely on
    # host Python or an assumed set of filenames in a mutable package index.
    with tarfile.open(fileobj=io.BytesIO(gzip.decompress(download('APKINDEX.tar.gz'))),
                      mode='r:', ignore_zeros=True) as archive:
        index = archive.extractfile('APKINDEX').read().decode()
    packages, providers = {}, {}
    for block in index.split('\n\n'):
        fields = dict(line.split(':', 1) for line in block.splitlines() if ':' in line)
        if 'P' not in fields:
            continue
        packages[fields['P']] = fields
        for provision in fields.get('p', '').split():
            providers[provision.split('=', 1)[0]] = fields['P']
    selected = {}
    pending = list(roots)
    while pending:
        requirement = pending.pop()
        if requirement.startswith('!'):
            continue
        requirement = re.split(r'[<>=~]', requirement)[0]
        name = providers.get(requirement, requirement)
        if name in selected:
            continue
        package = packages[name]
        selected[name] = package
        pending.extend(package.get('D', '').split())
    return selected


GUEST = r'''
import base64, glob, io, json, os, subprocess, tarfile, time
results=[]
for uid in (0, 1000):
    print('FERRY_VM_BEGIN', uid, flush=True)
    pid=os.fork()
    if pid == 0:
        if uid:
            os.setgroups([]); os.setgid(uid); os.setuid(uid)
        code=subprocess.call(['/usr/bin/python3','/test-workflow-sandbox.py','/ferry-sandbox'])
        os._exit(code)
    _, status=os.waitpid(pid,0)
    results.append({'uid':uid,'exit_code':os.waitstatus_to_exitcode(status)})
    print('FERRY_VM_CASE', json.dumps(results[-1]), flush=True)
buffer=io.BytesIO()
with tarfile.open(fileobj=buffer,mode='w:gz') as archive:
    for path in sorted(glob.glob('/tmp/workflow-sandbox-*')):
        archive.add(path,arcname=os.path.basename(path))
print('FERRY_EVIDENCE_BEGIN', flush=True)
encoded=base64.b64encode(buffer.getvalue()).decode()
for offset in range(0,len(encoded),120): print(encoded[offset:offset+120],flush=True)
print('FERRY_EVIDENCE_END', flush=True)
print('FERRY_VM_RESULT '+json.dumps(results),flush=True)
print('FERRY_VM_DONE',flush=True)
'''


def main():
    qemu, kernel, original_initrd, binary = (Path(arg).resolve() for arg in sys.argv[1:])
    work = Path(tempfile.mkdtemp(prefix='workflow-linux-vm-'))
    print('Linux VM evidence:', work, flush=True)
    entries = read_cpio(gzip.decompress(original_initrd.read_bytes()))
    provenance = {}
    for name, package in alpine_packages().items():
        filename = f"{name}-{package['V']}.apk"
        data = download(filename)
        provenance[filename] = hashlib.sha256(data).hexdigest()
        # APK v2 is concatenated gzip/tar signature, control and data archives.
        with tarfile.open(fileobj=io.BytesIO(gzip.decompress(data)), mode='r:',
                          ignore_zeros=True) as archive:
            for member in archive:
                name = member.name.removeprefix('./').rstrip('/')
                if not name or name.startswith('.'):
                    continue
                assert not name.startswith('/') and '..' not in name.split('/')
                if member.isfile():
                    entries[name] = (stat.S_IFREG | member.mode, archive.extractfile(member).read(), 0, 0)
                elif member.issym():
                    entries[name] = (stat.S_IFLNK | member.mode, member.linkname.encode(), 0, 0)
                elif member.isdir():
                    entries[name] = (stat.S_IFDIR | member.mode, b'', 0, 0)
    init = '''#!/bin/busybox sh
/bin/busybox --install -s /bin
mount -t proc proc /proc
mount -t sysfs sysfs /sys
mount -t devtmpfs devtmpfs /dev
chmod 755 /
mkdir -p /tmp
chmod 1777 /tmp
echo 'FERRY_KERNEL'
uname -a
/usr/bin/python3 /guest.py
poweroff -f
'''
    for name, content, mode in [
        ('init', init.encode(), 0o755), ('guest.py', GUEST.encode(), 0o644),
        ('test-workflow-sandbox.py', (REPO/'scripts/test-workflow-sandbox.py').read_bytes(), 0o644),
        ('ferry-sandbox', binary.read_bytes(), 0o755),
    ]:
        entries[name] = (stat.S_IFREG | mode, content, 0, 0)
    initrd = work / 'acceptance-initramfs.gz'
    initrd.write_bytes(write_cpio(entries))
    (work/'inputs.json').write_text(json.dumps({
        'kernel_sha256': hashlib.sha256(kernel.read_bytes()).hexdigest(),
        'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
        'qemu': str(qemu), 'accelerator': 'tcg', 'packages_sha256': provenance,
    }, indent=2), encoding='utf-8')
    command = [str(qemu), '-nodefaults', '-no-user-config', '-display', 'none',
        '-serial', 'stdio', '-nic', 'none', '-no-reboot', '-m', '512', '-smp', '1',
        '-accel', 'tcg,thread=single', '-kernel', str(kernel), '-initrd', str(initrd),
        '-append', 'console=ttyS0 rdinit=/init panic=-1 quiet']
    started = time.monotonic()
    process = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
        creationflags=subprocess.CREATE_NO_WINDOW if sys.platform == 'win32' else 0)
    try:
        output, _ = process.communicate(timeout=240)
    except subprocess.TimeoutExpired:
        process.kill()
        output, _ = process.communicate(timeout=10)
        (work/'serial.log').write_bytes(output)
        raise
    finally:
        if process.poll() is None:
            process.kill(); process.wait(timeout=10)
    (work/'serial.log').write_bytes(output)
    decoded = output.decode(errors='replace').replace('\r', '')
    print('\n'.join(line for line in decoded.splitlines() if line.startswith(('PASS', 'FERRY_VM_', 'Sandbox evidence:'))), flush=True)
    if 'FERRY_EVIDENCE_BEGIN\n' in decoded:
        encoded = decoded.split('FERRY_EVIDENCE_BEGIN\n', 1)[1].split('FERRY_EVIDENCE_END', 1)[0]
        (work/'guest-evidence.tar.gz').write_bytes(base64.b64decode(encoded))
    assert 'FERRY_VM_DONE' in decoded, (process.returncode, str(work/'serial.log'))
    result = json.loads(decoded.split('FERRY_VM_RESULT ', 1)[1].splitlines()[0])
    (work/'result.json').write_text(json.dumps({'results': result,
        'elapsed_seconds': time.monotonic()-started, 'accelerator': 'tcg'}, indent=2), encoding='utf-8')
    assert all(row['exit_code'] == 0 for row in result), result
    print('PASS Linux root + user-namespace sandbox acceptance', flush=True)


if __name__ == '__main__':
    main()
