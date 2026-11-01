#!/bin/sh
# One-command single-host Docker Compose deployment:
#   curl -fsSL https://raw.githubusercontent.com/7link-team/ferry/main/scripts/quickstart.sh | sh
#   curl -fsSL .../quickstart.sh | sh -s -- --database mysql --public-url https://ops.example.com
# Creates (or reuses) a deployment directory with the released Compose files of
# one version and random credentials, then starts the stack and prints the
# first-time setup token. Re-running reuses every secret and address.
set -eu

# Everything runs from main, invoked on the last line: when piped from curl, a
# truncated download defines nothing runnable instead of executing part of it.
main() {
repository=${FERRY_REPOSITORY:-7link-team/ferry}
image_repository=${FERRY_IMAGE_REPOSITORY:-ghcr.io/7link-team/ferry-server}
version=
database=postgres
public_url=
bind=127.0.0.1
port=4290
redis=1
init_only=
directory=ferry

fail() { printf 'Quickstart failed: %s\n' "$*" >&2; exit 1; }
usage() {
    cat <<'USAGE'
Usage: quickstart.sh [options]
  --dir DIR             Deployment directory (default: ./ferry)
  --version vX.Y.Z      Release to deploy (default: latest published release)
  --database NAME       postgres (default) or mysql; fixed at first start
  --public-url URL      Externally reachable HTTP(S) origin (default: http://localhost:PORT)
  --bind ADDRESS        Published listener address (default: 127.0.0.1; 0.0.0.0 for LAN)
  --port PORT           Published port (default: 4290)
  --without-redis       Do not start the optional Redis service
  --init-only           Create configuration without running Docker
USAGE
}

value() { [ "$#" -ge 2 ] && [ -n "$2" ] || fail "missing value for $1"; }
while [ "$#" -gt 0 ]; do
    case "$1" in
        --dir) value "$@"; directory=$2; shift 2 ;;
        --version) value "$@"; version=$2; shift 2 ;;
        --database) value "$@"; database=$2; shift 2 ;;
        --public-url) value "$@"; public_url=$2; shift 2 ;;
        --bind) value "$@"; bind=$2; shift 2 ;;
        --port) value "$@"; port=$2; shift 2 ;;
        --without-redis) redis=; shift ;;
        --init-only) init_only=1; shift ;;
        -h|--help) usage; exit 0 ;;
        *) usage >&2; fail "unknown option: $1" ;;
    esac
done

case "$database" in postgres|mysql) ;; *) fail "--database must be postgres or mysql" ;; esac
case "$port" in ''|*[!0-9]*) fail "--port must be a number" ;; esac
[ "${#port}" -le 5 ] && [ "$port" -ge 1 ] && [ "$port" -le 65535 ] || fail "--port must be between 1 and 65535"
[ -n "$public_url" ] || public_url="http://localhost:$port"
public_url=${public_url%/}
# An HTTP(S) origin only: host or [IPv6], optional port; no credentials, path, query or fragment.
printf '%s\n' "$public_url" | grep -Eq '^https?://([A-Za-z0-9.-]+|\[[0-9A-Fa-f:.]+\])(:[0-9]{1,5})?$' \
    || fail "--public-url must be an HTTP(S) origin without credentials, path, query or fragment"
printf '%s\n' "$bind" | grep -Eq '^[0-9A-Fa-f:.]+$' || fail "--bind must be an IP address"
case "$version" in ''|v[0-9]*) ;; *) fail "--version must look like v0.1.0" ;; esac
case "$version" in *[!A-Za-z0-9.+-]*) fail "--version contains unsupported characters" ;; esac
case "$repository$image_repository" in *[!A-Za-z0-9._/:-]*) fail "unsupported repository name" ;; esac

fetch() { # URL DESTINATION
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL --proto '=https' --retry 3 -o "$2" "$1"
    elif command -v wget >/dev/null 2>&1; then
        wget -q -O "$2" "$1"
    else
        fail "curl or wget is required"
    fi
}
random() { od -An -N32 -tx1 /dev/urandom | tr -d ' \n'; }

umask 077
mkdir -p "$directory/scripts"
cd "$directory"
env_file=.env.production
files="compose.production.yaml compose.postgres.yaml compose.mysql.yaml scripts/postgres-init.sh"

if [ -f "$env_file" ]; then
    grep -qx "FERRY_DATABASE_ENGINE=$database" "$env_file" \
        || fail "this deployment uses another database; switching requires a separate directory and a data migration"
fi

missing=
for file in $files; do [ -f "$file" ] || missing=1; done
if [ -n "$missing" ] || [ ! -f "$env_file" ]; then
    if [ -z "$version" ] && [ -f "$env_file" ]; then
        version=$(sed -n 's/^FERRY_IMAGE=.*:\(v[^:@]*\)$/\1/p' "$env_file" | head -n 1)
    fi
    if [ -z "$version" ]; then
        printf 'Resolving the latest Ferry release...\n'
        # The releases feed lists pre-releases newest first and is not subject
        # to the anonymous API rate limit; the API is only a fallback.
        if fetch "https://github.com/$repository/releases.atom" .releases 2>/dev/null; then
            # One XML tag per line, so the first (newest) release wins.
            version=$(tr '<' '\n' < .releases | sed -n 's#.*/releases/tag/\(v[^"]*\)".*#\1#p' | head -n 1)
        fi
        if [ -z "$version" ] && fetch "https://api.github.com/repos/$repository/releases?per_page=1" .releases 2>/dev/null; then
            version=$(sed -n 's/.*"tag_name": *"\(v[^"]*\)".*/\1/p' .releases | head -n 1)
        fi
        rm -f .releases
        case "$version" in v[0-9]*) ;; *) fail "could not determine the latest release; pass --version" ;; esac
    fi
    for file in $files; do
        if [ ! -f "$file" ]; then
            printf 'Downloading %s (%s)...\n' "$file" "$version"
            fetch "https://raw.githubusercontent.com/$repository/$version/$file" "$file.download" \
                || { rm -f "$file.download"; fail "cannot download $file for $version"; }
            mv "$file.download" "$file"
        fi
    done
    chmod 755 scripts/postgres-init.sh
fi

if [ ! -f "$env_file" ]; then
    host=${public_url#*://}
    case "$host" in
        \[*) host=${host%%]*}; host=${host#?} ;; # [IPv6]:port -> IPv6
        *:*) host=${host%:*} ;;
    esac
    secure=false
    case "$public_url" in https://*) secure=true ;; esac
    # Hex secrets are safe inside SQL and NATS URLs without encoding.
    {
        printf '# Keep this file and the data volumes together. Never regenerate the master key.\n'
        printf 'FERRY_DATABASE_ENGINE=%s\n' "$database"
        printf 'COMPOSE_PROJECT_NAME=ferry\n'
        printf 'FERRY_IMAGE=%s:%s\n' "$image_repository" "$version"
        printf 'FERRY_PUBLIC_URL=%s\n' "$public_url"
        printf 'FERRY_ALLOWED_HOSTS=%s,localhost,127.0.0.1\n' "$host"
        printf 'FERRY_SECURE_COOKIE=%s\n' "$secure"
        printf 'FERRY_BIND_ADDRESS=%s\n' "$bind"
        printf 'FERRY_PORT=%s\n' "$port"
        for key in FERRY_MASTER_KEY DATABASE_PASSWORD POSTGRES_ROOT_PASSWORD MYSQL_ROOT_PASSWORD NATS_PASSWORD REDIS_PASSWORD; do
            printf '%s=%s\n' "$key" "$(random)"
        done
    } > "$env_file.new"
    mv "$env_file.new" "$env_file"
    printf 'Created %s/%s with random credentials. Back up this file securely.\n' "$(pwd)" "$env_file"
else
    printf 'Reusing %s/%s unchanged; edit it to change addresses or the image version.\n' "$(pwd)" "$env_file"
fi

[ -z "$init_only" ] || exit 0
command -v docker >/dev/null 2>&1 && docker compose version >/dev/null 2>&1 \
    || fail "Docker with Compose v2 is required. Configuration is ready in $(pwd); install Docker and rerun this command."
set -- docker compose --env-file "$env_file" -f compose.production.yaml -f "compose.$database.yaml"
[ -z "$redis" ] || set -- "$@" --profile redis
"$@" config --quiet
"$@" up -d --wait --wait-timeout 300
printf 'Stack is healthy. Open the FERRY_PUBLIC_URL from %s/%s.\n' "$(pwd)" "$env_file"
printf 'First-time setup token (only present before initialization):\n'
"$@" exec -T ferry sh -c 'if [ -f /data/setup-token ]; then cat /data/setup-token; else echo "Already initialized."; fi'
}

main "$@"
