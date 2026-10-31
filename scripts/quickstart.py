#!/usr/bin/env python3
"""Initialize stable deployment credentials and start the Docker Compose stack."""
import argparse
import os
from pathlib import Path
import secrets
import shutil
import subprocess
import sys
from urllib.parse import urlsplit

ROOT = Path(__file__).resolve().parent.parent


def initialize(env_file, database, public_url, bind, port):
    if env_file.exists():
        content = env_file.read_text(encoding="utf-8")
        if f"FERRY_DATABASE_ENGINE={database}\n" not in content:
            raise ValueError("Existing deployment uses another database. Use a separate deployment directory; switching requires data migration.")
        return False
    url = urlsplit(public_url)
    if url.scheme not in ("http", "https") or not url.hostname or url.username or url.password or url.query or url.fragment or url.path not in ("", "/"):
        raise ValueError("--public-url must be an HTTP(S) origin without credentials, path, query or fragment.")
    if any(c in public_url + bind for c in '\r\n$"\\'):
        raise ValueError("Unsupported character in deployment address.")
    # Hex secrets are also safe inside SQL and NATS URLs without encoding.
    values = {
        "FERRY_DATABASE_ENGINE": database,
        "COMPOSE_PROJECT_NAME": "ferry",
        "FERRY_IMAGE": "ghcr.io/7link-team/ferry-server:v0.1.0-alpha.24",
        "FERRY_PUBLIC_URL": public_url.rstrip("/"),
        "FERRY_ALLOWED_HOSTS": f"{url.hostname},localhost,127.0.0.1",
        "FERRY_SECURE_COOKIE": str(url.scheme == "https").lower(),
        "FERRY_BIND_ADDRESS": bind,
        "FERRY_PORT": str(port),
        "FERRY_MASTER_KEY": secrets.token_hex(32),
        "DATABASE_PASSWORD": secrets.token_hex(32),
        "POSTGRES_ROOT_PASSWORD": secrets.token_hex(32),
        "MYSQL_ROOT_PASSWORD": secrets.token_hex(32),
        "NATS_PASSWORD": secrets.token_hex(32),
        "REDIS_PASSWORD": secrets.token_hex(32),
    }
    fd = os.open(env_file, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, "w", encoding="utf-8", newline="\n") as output:
        output.write("# Keep this file and the data volumes together. Never regenerate the master key.\n")
        for key, value in values.items():
            output.write(f"{key}={value}\n")
    return True


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--database", choices=("postgres", "mysql"), default="postgres")
    parser.add_argument("--public-url", help="Externally reachable HTTP(S) origin; defaults to localhost at --port")
    parser.add_argument("--bind", default="127.0.0.1", help="Published listener address; use 0.0.0.0 for LAN access")
    parser.add_argument("--port", type=int, default=4290)
    parser.add_argument("--without-redis", action="store_true")
    parser.add_argument("--init-only", action="store_true", help="Create configuration without running Docker")
    args = parser.parse_args()
    if not 1 <= args.port <= 65535:
        parser.error("--port must be between 1 and 65535")
    env_file = ROOT / ".env.production"
    if initialize(env_file, args.database, args.public_url or f"http://localhost:{args.port}", args.bind, args.port):
        print("Created .env.production with random credentials. Back up this file securely.")
    else:
        print("Reusing .env.production unchanged; edit it to change addresses or image versions.")
    if args.init_only:
        return
    docker = shutil.which("docker")
    if not docker:
        raise ValueError("Docker with Compose v2 is required. Configuration is ready; install Docker and rerun this command.")
    command = [docker, "compose", "--env-file", str(env_file), "-f", str(ROOT / "compose.production.yaml"), "-f", str(ROOT / f"compose.{args.database}.yaml")]
    if not args.without_redis:
        command += ["--profile", "redis"]
    options = {"cwd": ROOT, "check": True}
    if os.name == "nt":
        options["creationflags"] = subprocess.CREATE_NO_WINDOW
    subprocess.run(command + ["config", "--quiet"], **options)
    subprocess.run(command + ["up", "-d", "--wait", "--wait-timeout", "300"], **options)
    print("Stack is healthy. Open FERRY_PUBLIC_URL from .env.production.")
    print("First-time setup token (only present before initialization):")
    subprocess.run(command + ["exec", "-T", "ferry", "sh", "-c", 'if [ -f /data/setup-token ]; then cat /data/setup-token; else echo "Already initialized."; fi'], **options)


if __name__ == "__main__":
    try:
        main()
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        print(f"Quickstart failed: {error}", file=sys.stderr)
        sys.exit(1)
