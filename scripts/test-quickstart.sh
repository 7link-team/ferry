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
case "\$*" in *" \${DOCKER_FAIL:-none}"*) exit 1 ;; esac
case "\$*" in
    *pg_dump*|*mysqldump*) [ -n "\${DUMP_DOWN:-}" ] || echo '-- dump' ;;
    *" ps -a -q redis") [ -z "\${REDIS_UP:-}" ] || echo redis-container ;;
    *exec*) echo setup-token-123 ;;
esac
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

# --upgrade on the Postgres deployment created first (v9.9.9).
image_line() { grep '^FERRY_IMAGE=' "$1"; }
without_image() { grep -v '^FERRY_IMAGE=' "$1"; }
if run --dir absent --upgrade >/dev/null 2>&1; then echo 'FAIL upgrade without a deployment accepted'; exit 1; fi
check '[ ! -e "$work/absent" ]' 'upgrade refuses a directory without a deployment and creates nothing'
if run --upgrade --port 1234 >/dev/null 2>&1; then echo 'FAIL upgrade accepted a configuration option'; exit 1; fi
: > "$work/docker.log"
output=$(run --upgrade)
check 'printf "%s" "$output" | grep -q "Already on v9.9.9" && [ ! -s "$work/docker.log" ] && [ "$before" = "$(cat "$env")" ]' 'upgrade to the latest release already deployed changes nothing'
for older in v9.9.1 v9.9.9-rc.1; do
    if run --upgrade --version "$older" >/dev/null 2>&1; then echo "FAIL downgrade to $older accepted"; exit 1; fi
done
check '[ "$before" = "$(cat "$env")" ] && [ ! -e "$work/ferry/backups" ]' 'refuses downgrades, including to a pre-release of the deployed version'
(export DUMP_DOWN=1; run --upgrade --version v9.10.0 >/dev/null 2>&1) && { echo 'FAIL upgrade without a database backup'; exit 1; }
check '[ "$before" = "$(cat "$env")" ] && ! grep -q " pull" "$work/docker.log"' 'a failed database backup stops the upgrade before anything changes'
rm -rf "$work/ferry/backups"
: > "$work/docker.log"; : > "$work/curl.log"
output=$(run --upgrade --version v9.10.0)
check 'image_line "$env" | grep -qx "FERRY_IMAGE=ghcr.io/7link-team/ferry-server:v9.10.0" && [ "$(printf "%s\n" "$before" | grep -v ^FERRY_IMAGE=)" = "$(without_image "$env")" ]' 'upgrade moves FERRY_IMAGE to the new release and keeps every secret and address'
backup=$(ls -d "$work"/ferry/backups/*-v9.9.9-to-v9.10.0)
check '[ "$(cat "$backup/database.sql")" = "-- dump" ] && [ "$before" = "$(cat "$backup/.env.production")" ] && [ -f "$backup/compose.production.yaml" ] && [ -f "$backup/scripts/postgres-init.sh" ]' 'backs up the database dump, the env file and the Compose files of the old release'
check 'grep -q "exec -T postgres pg_dump -U postgres -d ferry" "$work/docker.log" && [ "$(grep -n -E "pg_dump| pull$|up -d --wait" "$work/docker.log" | cut -d: -f1 | tr "\n" " ")" = "$(grep -n -E "pg_dump| pull$|up -d --wait" "$work/docker.log" | cut -d: -f1 | sort -n | tr "\n" " ")" ] && grep -q "raw.githubusercontent.com/7link-team/ferry/v9.10.0/compose.production.yaml" "$work/curl.log"' 'dumps the database, then downloads the new release files, pulls and waits for health'
check '! grep -q -- "--profile redis config" "$work/docker.log" && [ ! -e "$work/ferry/.upgrade" ] && printf "%s" "$output" | grep -q "Upgraded from v9.9.9 to v9.10.0"' 'keeps Redis off when it was not deployed and leaves no staging files'
upgraded=$(cat "$env")
if (export DOCKER_FAIL=pull; run --upgrade --version v9.11.0 >/dev/null 2>&1); then echo 'FAIL failed pull reported success'; exit 1; fi
check '[ "$upgraded" = "$(cat "$env")" ]' 'a failed image pull restores the previous files'
: > "$work/docker.log"
(export REDIS_UP=1; run --upgrade --version v10.0.0 >/dev/null)
check 'grep -q -- "--profile redis pull" "$work/docker.log"' 'keeps Redis on when its container exists'
if (export DOCKER_FAIL='up -d'; run --upgrade --version v10.0.1 > /dev/null 2> "$work/err"); then echo 'FAIL unhealthy upgrade reported success'; exit 1; fi
check 'grep -q "did not become healthy" "$work/err" && grep -q "psql -U postgres -d ferry" "$work/err" && grep -q "v10.0.0-to-v10.0.1/database.sql" "$work/err"' 'an unhealthy release prints the rollback commands with its backup'
: > "$work/docker.log"
run --dir mysql --upgrade --version v9.10.0 >/dev/null
check 'grep -q "exec -T mysql sh -c MYSQL_PWD=.* mysqldump" "$work/docker.log" && grep -q "compose.mysql.yaml pull" "$work/docker.log" && grep -qx "FERRY_DATABASE_ENGINE=mysql" "$work/mysql/.env.production"' 'a MySQL deployment is backed up with mysqldump and upgraded in place'
(cd "$work" && sh -s -- --dir mysql --upgrade --version v1.0.0 < "$repo/scripts/quickstart.sh" >/dev/null 2>&1) && { echo 'FAIL piped downgrade accepted'; exit 1; }
run --dir semver --version v1.0.0-alpha.9 --init-only >/dev/null
run --dir semver --upgrade --version v1.0.0-alpha.10 >/dev/null
run --dir semver --upgrade --version v1.0.0 >/dev/null
check 'image_line "$work/semver/.env.production" | grep -q ":v1.0.0$"' 'orders pre-releases numerically (alpha.9 < alpha.10 < 1.0.0)'
(cd "$work" && sh -s -- --dir piped --version v9.9.7 --init-only < "$repo/scripts/quickstart.sh" >/dev/null)
check 'grep -qx "FERRY_IMAGE=ghcr.io/7link-team/ferry-server:v9.9.7" "$work/piped/.env.production"' 'works when piped to sh with arguments (curl ... | sh -s -- ...)'
half=$(($(wc -l < "$repo/scripts/quickstart.sh") / 2))
(cd "$work" && head -n "$half" "$repo/scripts/quickstart.sh" | sh -s -- --dir truncated --version v9.9.7 --init-only >/dev/null 2>&1) || true
check '[ ! -e "$work/truncated" ]' 'a truncated download executes nothing'
echo 'quickstart verified'
