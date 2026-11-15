"""Versioned Windows/Linux video example; Python 3.10+, standard library only."""
import argparse
import base64
import hashlib
import http.client
import io
import json
import os
from pathlib import Path
import platform
import socket
import ssl
import subprocess
import tarfile
import time
import urllib.parse
import urllib.request

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
OUTPUT = HERE / "output" / "video"


class MediaHTTPSConnection(http.client.HTTPSConnection):
    def connect(self):
        super().connect()
        # Large media/tool transfers otherwise suffer a small receive window
        # on Windows; keep the TLS verification performed by HTTPSConnection.
        self.sock.setsockopt(socket.SOL_SOCKET, socket.SO_RCVBUF, 1024 * 1024)


class MediaHTTPSHandler(urllib.request.HTTPSHandler):
    def https_open(self, request):
        return self.do_open(MediaHTTPSConnection, request, context=ssl.create_default_context())


def platform_key():
    architecture = platform.machine().lower()
    if architecture not in ("amd64", "x86_64"):
        raise RuntimeError(f"Unsupported architecture: {architecture}; expected x86_64")
    system = platform.system().lower()
    if system not in ("windows", "linux"):
        raise RuntimeError(f"Unsupported system: {system}; expected Windows or Linux")
    return system + "-x86_64"


def fetch(url, data=None, timeout=60):
    headers = {"Content-Type": "application/json"} if data is not None else {}
    request = urllib.request.Request(
        url, data=json.dumps(data).encode() if data is not None else None,
        headers=headers,
    )
    chunks, size = [], 0
    deadline = time.monotonic() + timeout
    with urllib.request.build_opener(MediaHTTPSHandler()).open(request, timeout=timeout) as response:
        while chunk := response.read(64 * 1024):
            if time.monotonic() > deadline:
                raise TimeoutError(f"Transfer exceeded {timeout} seconds")
            size += len(chunk)
            if size > 256 * 1024 * 1024:
                raise RuntimeError("Download exceeds 256 MiB")
            chunks.append(chunk)
    return b"".join(chunks)


def verified_package(content, integrity):
    actual = "sha512-" + base64.b64encode(hashlib.sha512(content).digest()).decode()
    if actual != integrity:
        raise RuntimeError("Tool package integrity mismatch")


def invoke(arguments, timeout=90):
    flags = {"creationflags": subprocess.CREATE_NO_WINDOW} if os.name == "nt" else {}
    result = subprocess.run(
        [str(argument) for argument in arguments], capture_output=True,
        timeout=timeout, **flags,
    )
    if result.returncode:
        raise RuntimeError(result.stderr.decode("utf-8", errors="replace")[-4000:])
    return result


def prepare():
    key = platform_key()
    lock = json.loads((HERE / "tools.lock.json").read_text(encoding="utf-8"))[key]
    directory = HERE / ".tools" / key
    directory.mkdir(parents=True, exist_ok=True)
    provenance = {"platform": key, "python": platform.python_version(), "tools": {}}
    for tool, package in lock.items():
        name = tool + (".exe" if os.name == "nt" else "")
        print("TOOL_DOWNLOADING", tool, package["version"], flush=True)
        content = fetch(package["url"])
        verified_package(content, package["integrity"])
        with tarfile.open(fileobj=io.BytesIO(content), mode="r:gz") as archive:
            member = archive.getmember("package/" + name)
            if not member.isfile():
                raise RuntimeError("Media tool is not a regular file")
            binary = archive.extractfile(member).read()
            executable = directory / name
            executable.write_bytes(binary)
            executable.chmod(0o755)
        version = invoke([executable, "-version"]).stdout.decode().splitlines()[0]
        provenance["tools"][tool] = {
            **package, "sha256": hashlib.sha256(binary).hexdigest(), "binary_version": version,
        }
        print("TOOL_READY", tool, version, flush=True)
    (directory / "provenance.json").write_text(
        json.dumps(provenance, indent=2), encoding="utf-8",
    )


def tools():
    directory = HERE / ".tools" / platform_key()
    provenance = json.loads((directory / "provenance.json").read_text(encoding="utf-8"))
    lock = json.loads((HERE / "tools.lock.json").read_text(encoding="utf-8"))[platform_key()]
    paths = {}
    for tool, package in lock.items():
        executable = directory / (tool + (".exe" if os.name == "nt" else ""))
        record = provenance["tools"][tool]
        if any(record[field] != package[field] for field in package):
            raise RuntimeError("Prepared tool does not match lock; run prepare again")
        if hashlib.sha256(executable.read_bytes()).hexdigest() != record["sha256"]:
            raise RuntimeError("Prepared tool integrity mismatch; run prepare again")
        paths[tool] = executable
    return paths, provenance


def probe(executable, path):
    return json.loads(invoke([
        executable, "-v", "error", "-show_streams", "-show_format", "-of", "json", path,
    ]).stdout)


def check_media(media):
    videos = [stream for stream in media["streams"] if stream["codec_type"] == "video"]
    if len(videos) != 1 or any(s["codec_type"] == "audio" for s in media["streams"]):
        raise RuntimeError("Expected one video stream and no audio")
    video = videos[0]
    if (video["width"], video["height"], video["codec_name"]) != (1280, 720, "h264"):
        raise RuntimeError("Expected 1280x720 H.264")
    if video["avg_frame_rate"] != "30/1" or int(video["nb_frames"]) != 150:
        raise RuntimeError("Expected 30 fps and 150 frames")
    if abs(float(media["format"]["duration"]) - 5) >= 0.001:
        raise RuntimeError("Expected exactly five seconds")


def generate():
    executables, provenance = tools()
    backend = os.environ.get("VIDEO_BACKEND", "").rstrip("/")
    url = urllib.parse.urlsplit(backend)
    if url.scheme != "https" or not url.netloc or url.username or url.query or url.fragment:
        raise RuntimeError("VIDEO_BACKEND must be an HTTPS backend URL without credentials/query/fragment")
    OUTPUT.mkdir(parents=True, exist_ok=True)
    report = {
        "result": "RUNNING", "backend": backend, "toolchain": provenance,
        "source_commit": invoke(["git", "-C", ROOT, "rev-parse", "HEAD"]).stdout.decode().strip(),
        "started_at": time.time(),
    }
    try:
        prompt = (HERE / "prompt.txt").read_text(encoding="utf-8").strip()
        payload = {"data": [
            prompt, "worst quality, inconsistent motion, blurry, jittery, distorted",
            None, None, 720, 1280, "text-to-video", 5.0, 9, 42, False, 1.0, True,
        ]}
        report["payload"] = payload
        submission = json.loads(fetch(backend + "/gradio_api/call/text_to_video", payload))
        report["event_id"] = submission["event_id"]
        print("VIDEO_SUBMITTED", report["event_id"], flush=True)
        events = fetch(
            backend + "/gradio_api/call/text_to_video/" + urllib.parse.quote(report["event_id"], safe=""),
            timeout=240,
        ).decode()
        report["events"] = events
        completion = next((
            json.loads(line[6:]) for line in events.splitlines()
            if line.startswith("data: [") and '"video"' in line
        ), None)
        if completion is None:
            raise RuntimeError("Backend returned no video: " + events[-2000:])
        media_url = completion[0]["video"]["url"]
        if not media_url.startswith(backend + "/gradio_api/file="):
            raise RuntimeError("Unexpected media URL from backend")
        original = OUTPUT / "backend-original.mp4"
        original.write_bytes(fetch(media_url))
        report["original_probe"] = probe(executables["ffprobe"], original)
        final = OUTPUT / "short-film.mp4"
        converted = invoke([
            executables["ffmpeg"], "-y", "-i", original, "-t", "5", "-an",
            "-vf", "scale=1280:720,fps=30", "-c:v", "libx264", "-pix_fmt", "yuv420p",
            "-preset", "fast", "-crf", "18", "-movflags", "+faststart", final,
        ])
        (OUTPUT / "ffmpeg.log").write_bytes(converted.stderr)
        media = probe(executables["ffprobe"], final)
        check_media(media)
        report["final_probe"] = media
        report["artifact"] = {
            "path": "examples/video-pipeline/output/video/short-film.mp4", "bytes": final.stat().st_size,
            "sha256": hashlib.sha256(final.read_bytes()).hexdigest(),
        }
        report["result"] = "PASS"
        print("VIDEO_GENERATED", json.dumps(report["artifact"]), flush=True)
    except Exception as error:
        report["result"] = "FAIL"
        report["error"] = str(error)
        print("VIDEO_FAILED", report["error"], flush=True)
        raise
    finally:
        report["elapsed_seconds"] = round(time.time() - report["started_at"], 3)
        (OUTPUT / "result.json").write_text(json.dumps(report, indent=2), encoding="utf-8")


def verify():
    executables, _ = tools()
    final = OUTPUT / "short-film.mp4"
    media = probe(executables["ffprobe"], final)
    check_media(media)
    # Decode every frame as well as checking container metadata.
    invoke([executables["ffmpeg"], "-v", "error", "-xerror", "-i", final, "-f", "null", "-"])
    report = json.loads((OUTPUT / "result.json").read_text(encoding="utf-8"))
    if report["result"] != "PASS" or hashlib.sha256(final.read_bytes()).hexdigest() != report["artifact"]["sha256"]:
        raise RuntimeError("Artifact does not match successful generation record")
    print("VIDEO_VERIFIED", json.dumps(report["artifact"]), flush=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("prepare", "generate", "verify"))
    arguments = parser.parse_args()
    try:
        {"prepare": prepare, "generate": generate, "verify": verify}[arguments.command]()
    except Exception as error:
        OUTPUT.mkdir(parents=True, exist_ok=True)
        record = OUTPUT / "result.json"
        report = json.loads(record.read_text(encoding="utf-8")) if record.exists() else {}
        report.update(result="FAIL", failed_stage=arguments.command, error=str(error))
        record.write_text(json.dumps(report, indent=2), encoding="utf-8")
        raise
