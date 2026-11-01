#!/bin/sh
# Run scripts/quickstart.sh for real against stub curl/docker: downloads come
# from this working tree, the release API returns v9.9.9, docker records argv.
set -eu
repo=$(cd "$(dirname "$0")/.." && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
mkdir "$work/bin"
cat > "$work/bin/curl" <<EOF
#!/bin/sh
out= url=
while [ "\$#" -gt 0 ]; do
    case "\$1" in -o) out=\$2; shift 2 ;; -*) shift ;; *) url=\$1; shift ;; esac
done
printf '%s\n' "\$url" >> "$work/curl.log"
case "\$url" in
    https://github.com/*/releases.atom) [ -z "\${ATOM_DOWN:-}" ] || exit 22; printf '<feed><entry><link rel="alternate" href="https://github.com/o/r/releases/tag/v9.9.9"/></entry><entry><link href="https://github.com/o/r/releases/tag/v9.9.1"/></entry></feed>' > "\$out" ;;
    https://api.github.com/repos/*/releases*) printf '[{"tag_name": "v9.9.6", "prerelease": true}]' > "\$out" ;;
    https://raw.githubusercontent.com/*) cp "$repo/\$(printf '%s' "\$url" | sed 's#^https://raw.githubusercontent.com/[^/]*/[^/]*/[^/]*/##')" "\$out" ;;
    *) exit 22 ;;
esac
EOF
cat > "$work/bin/docker" <<EOF
#!/bin/sh
printf '%s\n' "\$*" >> "$work/docker.log"
case "\$*" in *exec*) echo setup-token-123 ;; esac
EOF
chmod +x "$work/bin/curl" "$work/bin/docker"
PATH="$work/bin:$PATH"
export PATH
run() { (cd "$work" && sh "$repo/scripts/quickstart.sh" "$@"); }
check() { if eval "$1"; then printf 'PASS %s\n' "$2"; else printf 'FAIL %s\n' "$2"; exit 1; fi; }
env="$work/ferry/.env.production"

output=$(run --port 4399)
check '[ -f "$work/ferry/compose.production.yaml" ] && [ -f "$work/ferry/compose.postgres.yaml" ] && [ -f "$work/ferry/compose.mysql.yaml" ] && [ -f "$work/ferry/scripts/postgres-init.sh" ]' 'downloads every Compose file of the release into the deployment directory'
check 'cmp -s "$work/ferry/compose.production.yaml" "$repo/compose.production.yaml"' 'downloaded files are the release files byte for byte'
check 'grep -qx "FERRY_IMAGE=ghcr.io/7link-team/ferry-server:v9.9.9" "$env"' 'deploys the image of the latest published release, never a stale pin'
check 'grep -q "/releases.atom$" "$work/curl.log" && ! grep -q "api.github.com" "$work/curl.log"' 'resolves the newest release from the releases feed without the rate-limited API'
check 'grep -qx "FERRY_PUBLIC_URL=http://localhost:4399" "$env" && grep -qx "FERRY_SECURE_COOKIE=false" "$env" && grep -qx "FERRY_PORT=4399" "$env"' 'records the local address and port'
secrets=$(grep -E '^(FERRY_MASTER_KEY|DATABASE_PASSWORD|POSTGRES_ROOT_PASSWORD|MYSQL_ROOT_PASSWORD|NATS_PASSWORD|REDIS_PASSWORD)=' "$env" | cut -d= -f2)
check '[ "$(printf "%s\n" "$secrets" | grep -cE "^[0-9a-f]{64}$")" = 6 ] && [ "$(printf "%s\n" "$secrets" | sort -u | wc -l | tr -d " ")" = 6 ]' 'generates six distinct 256-bit hex secrets'
check 'sed -n 1p "$work/docker.log" | grep -q "compose version" && grep -q "compose.postgres.yaml --profile redis config --quiet" "$work/docker.log" && grep -q "up -d --wait --wait-timeout 300" "$work/docker.log"' 'validates the configuration before starting and waits for health'
check 'printf "%s" "$output" | grep -q setup-token-123' 'prints the first-time setup token'
before=$(cat "$env")
: > "$work/curl.log"
run --public-url https://other.example --port 1234 >/dev/null
check '[ "$before" = "$(cat "$env")" ] && [ ! -s "$work/curl.log" ]' 'rerun reuses every secret and address and needs no network'
if run --database mysql >/dev/null 2>&1; then echo 'FAIL database switch accepted'; exit 1; fi
check '[ "$before" = "$(cat "$env")" ]' 'refuses to switch an existing deployment to another database'

for url in "file:///tmp/x" "https://user:secret@example.com" "https://example.com/path" "https://example.com?x=1" "https://example.com/#x" 'http://example.com/$HOME'; do
    if run --dir bad --public-url "$url" >/dev/null 2>&1; then echo "FAIL accepted $url"; exit 1; fi
done
check '[ ! -e "$work/bad" ]' 'rejects unsafe origins before creating anything'

: > "$work/docker.log"
run --dir secure --public-url https://ops.example.com --without-redis --version v9.9.8 >/dev/null
check 'grep -qx "FERRY_SECURE_COOKIE=true" "$work/secure/.env.production" && grep -qx "FERRY_ALLOWED_HOSTS=ops.example.com,localhost,127.0.0.1" "$work/secure/.env.production"' 'an HTTPS origin enables secure cookies and allows its host'
check 'grep -qx "FERRY_IMAGE=ghcr.io/7link-team/ferry-server:v9.9.8" "$work/secure/.env.production" && ! grep -q "api.github.com" "$work/curl.log"' 'an explicit --version is used without querying releases'
check '! grep -q -- "--profile redis" "$work/docker.log"' '--without-redis omits the Redis profile'

: > "$work/docker.log"
run --dir mysql --database mysql --public-url 'http://[fd00::10]:4290' --bind 0.0.0.0 --init-only >/dev/null
check 'grep -qx "FERRY_DATABASE_ENGINE=mysql" "$work/mysql/.env.production" && grep -qx "FERRY_ALLOWED_HOSTS=fd00::10,localhost,127.0.0.1" "$work/mysql/.env.production" && grep -qx "FERRY_BIND_ADDRESS=0.0.0.0" "$work/mysql/.env.production"' 'MySQL, IPv6 origin and LAN bind are recorded'
check '[ ! -s "$work/docker.log" ]' '--init-only never runs Docker'
: > "$work/curl.log"
(export ATOM_DOWN=1; run --dir fallback --init-only >/dev/null)
check 'grep -qx "FERRY_IMAGE=ghcr.io/7link-team/ferry-server:v9.9.6" "$work/fallback/.env.production"' 'falls back to the release API when the feed is unavailable'
(cd "$work" && sh -s -- --dir piped --version v9.9.7 --init-only < "$repo/scripts/quickstart.sh" >/dev/null)
check 'grep -qx "FERRY_IMAGE=ghcr.io/7link-team/ferry-server:v9.9.7" "$work/piped/.env.production"' 'works when piped to sh with arguments (curl ... | sh -s -- ...)'
half=$(($(wc -l < "$repo/scripts/quickstart.sh") / 2))
(cd "$work" && head -n "$half" "$repo/scripts/quickstart.sh" | sh -s -- --dir truncated --version v9.9.7 --init-only >/dev/null 2>&1) || true
check '[ ! -e "$work/truncated" ]' 'a truncated download executes nothing'
echo 'quickstart verified'
