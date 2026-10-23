#!/usr/bin/env python3
"""A WebSocket client with no dependencies, for the checks curl cannot make.

Two things in this system only speak WebSocket: the browser's terminal and the
runner's own connections. Proving a pty echoes, and proving the server refuses
a forged `Hello`, both need a client that can set a Cookie header and send a
raw frame — which the stdlib can do in about a hundred lines, and which no
third-party package in this repo provides.

  ws-probe.py frame    <url> <json>            send one text frame, print the answer
  ws-probe.py terminal <url> <cookie>          drive a pty and print what it echoed
"""

import base64
import json
import os
import socket
import struct
import sys
import time
from urllib.parse import urlparse

TEXT, BINARY, CLOSE, PING, PONG = 0x1, 0x2, 0x8, 0x9, 0xA


class Ws:
    def __init__(self, url, cookie=None):
        parsed = urlparse(url)
        if parsed.scheme != "ws":
            raise SystemExit(f"only ws:// is supported here, got {url}")
        host = parsed.hostname
        port = parsed.port or 80
        path = parsed.path + ("?" + parsed.query if parsed.query else "")
        self.sock = socket.create_connection((host, port), timeout=15)
        key = base64.b64encode(os.urandom(16)).decode()
        request = [
            f"GET {path} HTTP/1.1",
            f"Host: {host}:{port}",
            "Upgrade: websocket",
            "Connection: Upgrade",
            f"Sec-WebSocket-Key: {key}",
            "Sec-WebSocket-Version: 13",
        ]
        if cookie:
            request.append(f"Cookie: {cookie}")
        self.sock.sendall(("\r\n".join(request) + "\r\n\r\n").encode())

        buffered = b""
        while b"\r\n\r\n" not in buffered:
            chunk = self.sock.recv(4096)
            if not chunk:
                raise SystemExit("server closed during the handshake")
            buffered += chunk
        head, rest = buffered.split(b"\r\n\r\n", 1)
        status = head.split(b"\r\n")[0].decode(errors="replace")
        if "101" not in status:
            raise SystemExit(f"handshake refused: {status}\n{head.decode(errors='replace')}")
        self.buffer = bytearray(rest)

    def send(self, opcode, payload):
        mask = os.urandom(4)
        masked = bytes(b ^ mask[i % 4] for i, b in enumerate(payload))
        size = len(payload)
        if size < 126:
            header = struct.pack("!BB", 0x80 | opcode, 0x80 | size)
        elif size < 65536:
            header = struct.pack("!BBH", 0x80 | opcode, 0x80 | 126, size)
        else:
            header = struct.pack("!BBQ", 0x80 | opcode, 0x80 | 127, size)
        self.sock.sendall(header + mask + masked)

    def text(self, value):
        self.send(TEXT, value.encode())

    def _fill(self, size, deadline):
        while len(self.buffer) < size:
            self.sock.settimeout(max(0.05, deadline - time.time()))
            chunk = self.sock.recv(65536)
            if not chunk:
                return False
            self.buffer += chunk
        return True

    def frames(self, seconds):
        """Yields (opcode, payload) until the time runs out or the peer closes."""
        deadline = time.time() + seconds
        while time.time() < deadline:
            try:
                if not self._fill(2, deadline):
                    return
                first, second = self.buffer[0], self.buffer[1]
                length, offset = second & 0x7F, 2
                if length == 126:
                    if not self._fill(4, deadline):
                        return
                    length = struct.unpack("!H", self.buffer[2:4])[0]
                    offset = 4
                elif length == 127:
                    if not self._fill(10, deadline):
                        return
                    length = struct.unpack("!Q", self.buffer[2:10])[0]
                    offset = 10
                if not self._fill(offset + length, deadline):
                    return
                payload = bytes(self.buffer[offset : offset + length])
                del self.buffer[: offset + length]
                opcode = first & 0x0F
                if opcode == PING:
                    self.send(PONG, payload)
                    continue
                if opcode == CLOSE:
                    return
                if opcode == PONG:
                    continue
                yield opcode, payload
            except (TimeoutError, socket.timeout):
                return

    def close(self):
        try:
            self.send(CLOSE, b"")
        except OSError:
            pass
        self.sock.close()


def one_frame(url, payload):
    """Sends a single text frame and prints the first text frame that comes back."""
    ws = Ws(url)
    ws.text(payload)
    for opcode, data in ws.frames(10):
        if opcode == TEXT:
            print(data.decode(errors="replace"))
            ws.close()
            return
    ws.close()
    print(json.dumps({"error": "no答复，连接已关闭"}, ensure_ascii=False))


def terminal(url, cookie):
    """Types into a pty, resizes it, and prints everything it echoed back.

    `stty size` is the check that a resize reached the kernel rather than just
    the wire: the shell asks the tty itself what size it is.
    """
    ws = Ws(url, cookie)
    collected = bytearray()

    def drain(seconds):
        for opcode, data in ws.frames(seconds):
            if opcode == BINARY:
                collected.extend(data)
            elif opcode == TEXT:
                collected.extend(b"\n[text] " + data + b"\n")

    drain(1.5)
    ws.send(BINARY, b"echo hello\n")
    drain(2.0)
    ws.text(json.dumps({"type": "resize", "cols": 111, "rows": 37}))
    time.sleep(0.4)
    ws.send(BINARY, b"stty size\n")
    drain(2.5)
    ws.close()
    sys.stdout.write(collected.decode("utf-8", errors="replace"))


if __name__ == "__main__":
    if len(sys.argv) >= 3 and sys.argv[1] == "frame":
        one_frame(sys.argv[2], sys.argv[3])
    elif len(sys.argv) >= 3 and sys.argv[1] == "terminal":
        terminal(sys.argv[2], sys.argv[3] if len(sys.argv) > 3 else None)
    else:
        raise SystemExit(__doc__)
