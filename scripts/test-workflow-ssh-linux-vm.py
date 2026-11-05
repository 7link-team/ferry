#!/usr/bin/env python3
"""Run the SSH workflow gate against an ephemeral Linux VM, without Docker/installing sshd."""
import gzip
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import socket
import stat
import subprocess
import sys
import tarfile
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('workflow_vm', ROOT/'scripts/test-workflow-sandbox-linux-vm.py')
vm = importlib.util.module_from_spec(spec)
spec.loader.exec_module(vm)
tools = Path(os.environ.get('FERRY_QEMU_TOOLS', ROOT/'worktree/workflow-platform/data/maintenance/linux-vm-tools'))
assert (tools/'qemu/qemu-system-x86_64.exe').is_file(), 'QEMU tool directory must contain the previously verified Linux VM tools'
vm.TOOLS = ROOT/'data/maintenance/ssh-linux-vm-tools'
fetch_package = vm.download
def download(name):
    cached = tools/'packages'/name
    return cached.read_bytes() if cached.is_file() else fetch_package(name)
vm.download = download
work = Path(tempfile.mkdtemp(prefix='workflow-ssh-linux-'))
print('SSH Linux evidence:', work, flush=True)
entries = vm.read_cpio(gzip.decompress((tools/'initramfs-virt').read_bytes()))
provenance = {}
for name, package in vm.alpine_packages(['busybox', 'openssh-server', 'openssh-sftp-server', 'openssh-client-default', 'git']).items():
    filename = f"{name}-{package['V']}.apk"
    data = download(filename)
    provenance[filename] = hashlib.sha256(data).hexdigest()
    with tarfile.open(fileobj=io.BytesIO(gzip.decompress(data)), mode='r:', ignore_zeros=True) as archive:
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
mkdir -p /tmp /run/sshd /root /etc/ssh /dev/pts /var/empty
mount -t devpts devpts /dev/pts
chmod 1777 /tmp
modprobe virtio_net
ifconfig lo up
ifconfig eth0 10.0.2.15 netmask 255.255.255.0 up
route add default gw 10.0.2.2
printf '%s\\n' 'root:x:0:0:root:/root:/bin/sh' 'sshd:x:22:22:sshd:/var/empty:/sbin/nologin' > /etc/passwd
printf '%s\\n' 'root:x:0:' 'sshd:x:22:' > /etc/group
echo 'root:!:19000:0:99999:7:::' > /etc/shadow
echo 'root:fixture-ssh-password' | chpasswd
ssh-keygen -q -t ed25519 -N '' -f /etc/ssh/ssh_host_ed25519_key
printf '%s\\n' 'HostKey /etc/ssh/ssh_host_ed25519_key' 'PasswordAuthentication yes' 'PermitRootLogin yes' 'Subsystem sftp /usr/lib/ssh/sftp-server' > /etc/ssh/sshd_config
git init /tmp/repository
git -C /tmp/repository config user.name 'SSH acceptance'
git -C /tmp/repository config user.email 'fixture@example.invalid'
echo 'Real Linux SSH workflow acceptance' > /tmp/repository/README
git -C /tmp/repository add README
git -C /tmp/repository commit -m 'SSH_WORKFLOW_REAL'
echo FERRY_SSH_LINUX_READY
exec /usr/sbin/sshd -D -e
'''
entries['init'] = (stat.S_IFREG | 0o755, init.encode(), 0, 0)
initrd = work/'ssh-initramfs.gz'
initrd.write_bytes(vm.write_cpio(entries))
with socket.socket() as listener:
    listener.bind(('127.0.0.1', 0))
    port = listener.getsockname()[1]
(work/'inputs.json').write_text(json.dumps({'kernel_sha256': hashlib.sha256((tools/'vmlinuz-virt').read_bytes()).hexdigest(), 'packages_sha256': provenance, 'accelerator': 'tcg', 'port': port}, indent=2))
command = [str(tools/'qemu/qemu-system-x86_64.exe'), '-nodefaults', '-no-user-config', '-display', 'none', '-serial', 'stdio', '-no-reboot', '-m', '256', '-smp', '1', '-accel', 'tcg,thread=single', '-kernel', str(tools/'vmlinuz-virt'), '-initrd', str(initrd), '-append', 'console=ttyS0 rdinit=/init panic=-1 quiet', '-netdev', f'user,id=net0,hostfwd=tcp:127.0.0.1:{port}-:22', '-device', 'virtio-net-pci,netdev=net0']
with (work/'serial.log').open('wb') as log:
    process = subprocess.Popen(command, stdout=log, stderr=subprocess.STDOUT, creationflags=subprocess.CREATE_NO_WINDOW if sys.platform == 'win32' else 0)
    try:
        for attempt in range(300):
            assert process.poll() is None, 'Linux VM exited before SSH was ready'
            try:
                with socket.create_connection(('127.0.0.1', port), timeout=.5) as connection:
                    connection.settimeout(.5)
                    if connection.recv(128).startswith(b'SSH-'):
                        break
            except (TimeoutError, OSError):
                pass
            time.sleep(.2)
        else:
            raise AssertionError('Linux SSH startup timeout; see serial.log')
        env = {**os.environ, 'FERRY_BOUNDARY_SSH_PORT': str(port)}
        result = subprocess.run(['node', 'scripts/test-workflow-host-boundaries.mjs'], cwd=ROOT, env=env, creationflags=subprocess.CREATE_NO_WINDOW if sys.platform == 'win32' else 0)
        assert result.returncode == 0, 'SSH workflow acceptance failed'
    finally:
        process.kill()
        process.wait(timeout=10)
