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
    https://github.com/*/releases/download/*/runner-update.json*)
        case "\$url" in */\${UNPUBLISHED_VERSION:-no-version}/*) exit 22 ;; esac
        case "\$url" in */\${UNSIGNED_VERSION:-no-version}/*.sig) exit 22 ;; esac
        printf 'fixture release availability marker\n' > "\$out" ;;
    https://raw.githubusercontent.com/*) [ -z "\${EMPTY_DOWNLOAD:-}" ] || exit 0; cp "$repo/\$(printf '%s' "\$url" | sed 's#^https://raw.githubusercontent.com/[^/]*/[^/]*/[^/]*/##')" "\$out" ;;
    *) exit 22 ;;
esac
EOF
cat > "$work/bin/docker" <<EOF
#!/bin/sh
printf '%s\n' "\$*" >> "$work/docker.log"
if [ -n "\${DOCKER_FAIL:-}" ]; then case "\$*" in *" \$DOCKER_FAIL"*) exit 1 ;; esac; fi
case "\$*" in
    *" ps -q ferry") [ ! -f .test-running-image ] || echo ferry-container ;;
    "inspect --format "*) [ ! -f .test-running-image ] || printf '%s|running|%s\n' "\$(cat .test-running-image)" "\${RUNNING_HEALTH:-healthy}" ;;
    *" up -d "*) sed -n 's/^FERRY_IMAGE=//p' .env.production > .test-running-image ;;
    *" pull") if [ -n "\${INTERRUPT_PULL:-}" ]; then kill -s "\$INTERRUPT_PULL" "\$PPID"; exit 143; fi ;;
    *pg_dump*|*mysqldump*) [ -n "\${DUMP_DOWN:-}" ] || echo '-- dump' ;;
    *" ps -a -q redis") [ -z "\${REDIS_UP:-}" ] || echo redis-container ;;
    "ps -a --filter label=com.docker.compose.project=ferry --format"*) [ -z "\${OTHER_DEPLOYMENT:-}" ] || echo "\$OTHER_DEPLOYMENT" ;;
    "ps -a -q --filter label=com.docker.compose.project=ferry") [ -z "\${OTHER_DEPLOYMENT:-}" ] || echo container-id ;;
    "volume ls -q --filter label=com.docker.compose.project=ferry") [ -z "\${LEFTOVER_VOLUMES:-}\${OTHER_DEPLOYMENT:-}" ] || echo ferry_postgres-data ;;
    *exec*) echo setup-token-123 ;;
esac
EOF
chmod +x "$work/bin/curl" "$work/bin/docker"
PATH="$work/bin:$PATH"
export PATH
run() { (cd "$work" && sh "$repo/scripts/quickstart.sh" "$@"); }
refused() { if run "$@" >/dev/null 2>&1; then echo "FAIL accepted: $*"; exit 1; fi; }
check() { if eval "$1"; then printf 'PASS %s\n' "$2"; else printf 'FAIL %s\n' "$2"; exit 1; fi; }
has() { grep -qx "$2" "$1"; }
secrets_of() { grep -E '^(FERRY_MASTER_KEY|DATABASE_PASSWORD|POSTGRES_ROOT_PASSWORD|MYSQL_ROOT_PASSWORD|NATS_PASSWORD|REDIS_PASSWORD)=' "$1"; }
env="$work/ferry/.env.production"

refused --migrate-storage --upgrade
refused --migrate-storage --init-only
refused --migrate-storage --version v9.9.9
refused --dir absent --migrate-storage

output=$(run --port 4399)
check '[ -f "$work/ferry/compose.production.yaml" ] && [ -f "$work/ferry/compose.postgres.yaml" ] && [ -f "$work/ferry/compose.mysql.yaml" ] && [ -f "$work/ferry/scripts/postgres-init.sh" ]' 'downloads every Compose file of the release into the deployment directory'
check 'cmp -s "$work/ferry/compose.production.yaml" "$repo/compose.production.yaml" && [ ! -e "$work/ferry/compose.https.yaml" ] && [ ! -e "$work/ferry/caddy/Caddyfile" ]' 'downloaded files are the release files byte for byte; no HTTPS proxy without a domain'
check 'has "$env" "FERRY_IMAGE=ghcr.io/7link-team/ferry-server:v9.9.9"' 'deploys the image of the latest published release, never a stale pin'
check 'grep -q "/releases.atom$" "$work/curl.log" && ! grep -q "api.github.com" "$work/curl.log"' 'resolves the newest release from the releases feed without the rate-limited API'
check 'has "$env" "FERRY_BIND_ADDRESS=0.0.0.0" && has "$env" "FERRY_ALLOWED_HOSTS=localhost,127.0.0.1" && has "$env" "FERRY_SECURE_COOKIE=false" && has "$env" "FERRY_PORT=4399" && has "$env" "FERRY_DOMAINS=" && has "$env" "FERRY_REDIS=true" && ! grep -q PUBLIC_URL "$env"' 'publishes on all interfaces with an explicit host allowlist, the port and Redis; no public URL setting'
secrets=$(secrets_of "$env" | cut -d= -f2)
check '[ "$(printf "%s\n" "$secrets" | grep -cE "^[0-9a-f]{64}$")" = 6 ] && [ "$(printf "%s\n" "$secrets" | sort -u | wc -l | tr -d " ")" = 6 ]' 'generates six distinct 256-bit hex secrets'
check 'grep -q "compose version" "$work/docker.log" && grep -q "compose.postgres.yaml --profile redis -f compose.storage.yaml config --quiet" "$work/docker.log" && grep -q "up -d --wait --wait-timeout 300" "$work/docker.log" && ! grep -q -- "--profile https" "$work/docker.log"' 'validates the configuration before starting and waits for health'
check 'printf "%s" "$output" | grep -q setup-token-123 && printf "%s" "$output" | grep -q "http://localhost:4399"' 'prints the address and the first-time setup token'
check 'has "$env" "FERRY_STORAGE_LAYOUT=directory" && grep -q "./data/ferry:/data" "$work/ferry/compose.storage.yaml" && grep -q "./data/postgres:/var/lib/postgresql/data" "$work/ferry/compose.storage.yaml" && ! grep -q "^volumes:" "$work/ferry/compose.storage.yaml"' 'new deployments bind persistent data inside their deployment directory'
check 'grep -q "chown 10001:10001" "$work/docker.log" && grep -q -- "--network none --mount type=bind" "$work/docker.log"' 'prepares non-root Ferry directory permissions using an isolated helper'
before=$(cat "$env")
: > "$work/curl.log"
run >/dev/null
check '[ "$before" = "$(cat "$env")" ] && [ ! -s "$work/curl.log" ]' 'rerun reuses every secret and address and needs no network'
refused --database mysql
check '[ "$before" = "$(cat "$env")" ]' 'refuses to switch an existing deployment to another database'

for name in https://ops.example.com ops.example.com:443 1.2.3.4 '*.example.com' 'exa mple.com' user@example.com localhost ops.example.com/x; do refused --dir bad --domain "$name"; done
for name in http://10.0.0.1 10.0.0.1:4290 'a b' '$HOME'; do refused --dir bad --allow-host "$name"; done
refused --dir bad --domain ops.example.com --allow-host 10.0.0.1
(export EMPTY_DOWNLOAD=1; refused --dir empty --version v9.9.9)
check '[ ! -e "$work/empty/.env.production" ] && [ -z "$(ls "$work/empty" | grep -v scripts)" ]' 'a download that reports success without a file stops before configuring anything'
check '[ ! -e "$work/bad" ]' 'rejects malformed domains and hosts, and HTTPS mixed with plain HTTP, before creating anything'

: > "$work/docker.log"
output=$(run --dir secure --domain OPS.example.com --domain ops.example.org --domain ops.example.com --without-redis --version v9.9.8)
senv="$work/secure/.env.production"
check 'has "$senv" "FERRY_DOMAINS=ops.example.com, ops.example.org" && has "$senv" "FERRY_ALLOWED_HOSTS=ops.example.com,ops.example.org,localhost,127.0.0.1" && has "$senv" "FERRY_SECURE_COOKIE=true"' 'several domains are recorded once each, lower-cased, allowed, with secure cookies'
check 'grep -qx "ops.example.com, ops.example.org {" "$work/secure/caddy/Caddyfile" && grep -q "reverse_proxy ferry:4290" "$work/secure/caddy/Caddyfile" && grep -q "caddy-data:/data" "$work/secure/compose.https.yaml" && grep -q "compose.postgres.yaml -f compose.https.yaml -f compose.storage.yaml up -d" "$work/docker.log" && ! grep -q -- "--profile redis" "$work/docker.log"' 'domains generate the HTTPS proxy for every site; --without-redis omits Redis'
check 'printf "%s" "$output" | grep -q "https://ops.example.com" && printf "%s" "$output" | grep -q "https://ops.example.org" && printf "%s" "$output" | grep -q "logs caddy"' 'prints every HTTPS address and where to look for certificate errors'
check 'has "$senv" "FERRY_IMAGE=ghcr.io/7link-team/ferry-server:v9.9.8" && ! grep -q "api.github.com" "$work/curl.log"' 'an explicit --version is used without querying releases'
ssecrets=$(secrets_of "$senv")
run --dir secure --domain new.example.net --init-only >/dev/null
check 'has "$senv" "FERRY_DOMAINS=new.example.net" && has "$senv" "FERRY_ALLOWED_HOSTS=new.example.net,localhost,127.0.0.1" && grep -qx "new.example.net {" "$work/secure/caddy/Caddyfile" && [ "$ssecrets" = "$(secrets_of "$senv")" ]' 'rerunning with --domain replaces the domains and the proxy sites, keeping every secret'
: > "$work/docker.log"
run --dir secure --domain other.example.net >/dev/null
check 'grep -q "compose.https.yaml -f compose.storage.yaml restart caddy" "$work/docker.log"' 'a running proxy restarts when its domains change'
: > "$work/docker.log"
run --dir secure --domain other.example.net >/dev/null
check '! grep -q "restart caddy" "$work/docker.log"' 'an unchanged domain list does not restart the proxy'
: > "$work/docker.log"
run --dir secure --allow-host 192.168.0.10 --bind 0.0.0.0 --port 8080 >/dev/null
check 'has "$senv" "FERRY_DOMAINS=" && has "$senv" "FERRY_SECURE_COOKIE=false" && has "$senv" "FERRY_ALLOWED_HOSTS=192.168.0.10,localhost,127.0.0.1" && has "$senv" "FERRY_BIND_ADDRESS=0.0.0.0" && has "$senv" "FERRY_PORT=8080" && [ "$ssecrets" = "$(secrets_of "$senv")" ]' 'switching to plain-HTTP LAN access turns HTTPS off and keeps every secret'
check 'grep -q "compose.https.yaml -f compose.storage.yaml rm -s -f caddy" "$work/docker.log" && [ ! -e "$work/secure/compose.https.yaml" ] && [ ! -e "$work/secure/caddy/Caddyfile" ] && ! grep -q "compose.https.yaml up" "$work/docker.log"' 'removing the domains stops and removes the HTTPS proxy'

: > "$work/docker.log"
run --dir mysql --database mysql --allow-host fd00::10 --bind 0.0.0.0 --init-only >/dev/null
check 'has "$work/mysql/.env.production" "FERRY_DATABASE_ENGINE=mysql" && has "$work/mysql/.env.production" "FERRY_ALLOWED_HOSTS=fd00::10,localhost,127.0.0.1" && has "$work/mysql/.env.production" "FERRY_BIND_ADDRESS=0.0.0.0"' 'MySQL, an IPv6 host and LAN bind are recorded'
check '! grep -qE "compose .*(config|up|pull|exec|rm|restart)" "$work/docker.log"' '--init-only only queries Docker and never starts or changes anything'
run --dir mysql --allow-host 10.0.0.2 --init-only >/dev/null
check 'has "$work/mysql/.env.production" "FERRY_DATABASE_ENGINE=mysql"' 'reruns of a MySQL deployment need no --database'
: > "$work/curl.log"
(export ATOM_DOWN=1; run --dir fallback --init-only >/dev/null)
check 'has "$work/fallback/.env.production" "FERRY_IMAGE=ghcr.io/7link-team/ferry-server:v9.9.6"' 'falls back to the release API when the feed is unavailable'
(export UNPUBLISHED_VERSION=v9.9.9; run --dir published-fallback --init-only >/dev/null)
check 'has "$work/published-fallback/.env.production" "FERRY_IMAGE=ghcr.io/7link-team/ferry-server:v9.9.1" && [ ! -e "$work/published-fallback/.release-catalog" ]' 'skips Atom tags whose Runner release assets have not been published'
(export UNSIGNED_VERSION=v9.9.9; run --dir signed-fallback --init-only >/dev/null)
check 'has "$work/signed-fallback/.env.production" "FERRY_IMAGE=ghcr.io/7link-team/ferry-server:v9.9.1"' 'does not auto-select a partially published catalog missing its signature'

# External TLS keeps the application HTTP origin and has no local TLS proxy.
run --dir tunnel --proxy-domain Ops.Example.COM --proxy-domain ops.example.org --init-only >/dev/null
tenv="$work/tunnel/.env.production"
tsecrets=$(secrets_of "$tenv")
check 'has "$tenv" "FERRY_DOMAINS=" && has "$tenv" "FERRY_ALLOWED_HOSTS=ops.example.com,ops.example.org,localhost,127.0.0.1" && has "$tenv" "FERRY_SECURE_COOKIE=true" && has "$tenv" "FERRY_BIND_ADDRESS=0.0.0.0" && [ ! -f "$work/tunnel/compose.https.yaml" ]' 'external HTTPS allows the proxy domains, enables secure cookies and does not start Caddy'
output=$(run --dir tunnel)
check 'printf "%s" "$output" | grep -q "https://ops.example.com" && [ "$tsecrets" = "$(secrets_of "$tenv")" ]' 'external HTTPS reruns retain the access settings and secrets and print the public HTTPS address'
run --dir tunnel --domain old.example.com --init-only >/dev/null
run --dir tunnel --proxy-domain new.example.com --bind 0.0.0.0 >/dev/null
check 'has "$tenv" "FERRY_ALLOWED_HOSTS=new.example.com,localhost,127.0.0.1" && has "$tenv" "FERRY_SECURE_COOKIE=true" && [ ! -f "$work/tunnel/compose.https.yaml" ] && [ "$tsecrets" = "$(secrets_of "$tenv")" ]' 'switching from Caddy to external HTTPS removes Caddy and preserves existing credentials'
run --dir tunnel --bind 127.0.0.1 --init-only >/dev/null
run --dir tunnel --proxy-domain new.example.com --init-only >/dev/null
check 'has "$tenv" "FERRY_BIND_ADDRESS=0.0.0.0"' 'external proxy setup also replaces an old loopback default without requiring --bind'
run --dir tunnel --proxy-domain new.example.com --bind 127.0.0.1 --init-only >/dev/null
check 'has "$tenv" "FERRY_BIND_ADDRESS=127.0.0.1"' 'an explicit external proxy listener overrides the public default'
refused --proxy-domain ops.example.com --domain other.example.com
refused --proxy-domain ops.example.com --allow-host other.example.com
refused --proxy-domain https://ops.example.com
refused --proxy-domain 127.0.0.1
refused --upgrade --proxy-domain ops.example.com

# A second directory must not take over another deployment's containers and volumes.
(export OTHER_DEPLOYMENT=/opt/ferry; run --dir second >/dev/null 2>"$work/err") && { echo 'FAIL second deployment accepted'; exit 1; }
check '[ ! -e "$work/second/.env.production" ] && grep -q "already runs from /opt/ferry; rerun this command in /opt" "$work/err"' 'refuses to create a second deployment over the containers of another directory, naming it'
(export LEFTOVER_VOLUMES=1; run --dir third >/dev/null 2>"$work/err") && { echo 'FAIL deployment over leftover volumes accepted'; exit 1; }
check '[ ! -e "$work/third/.env.production" ] && grep -q "data volumes of an earlier Ferry deployment exist" "$work/err"' 'refuses to create a deployment over the data volumes of an earlier one'
before_rerun=$(cat "$env")
(export OTHER_DEPLOYMENT=/root/ferry; run >/dev/null)
check '[ "$before_rerun" = "$(cat "$env")" ]' 'an existing deployment can still take its containers back from another directory'

# Explicit loopback bindings on existing installations must not be broadened by reruns.
run --dir loopback --bind 127.0.0.1 --init-only >/dev/null
run --dir loopback --init-only >/dev/null
check 'has "$work/loopback/.env.production" "FERRY_BIND_ADDRESS=127.0.0.1"' 'keeps the listener chosen by an existing installation'

# A deployment made by an earlier release (up to v0.1.0-alpha.29): its Compose
# file requires FERRY_PUBLIC_URL; no FERRY_REDIS or FERRY_DOMAINS yet.
run --dir legacy --init-only >/dev/null
lenv="$work/legacy/.env.production"
grep -v -e '^FERRY_REDIS=' -e '^FERRY_DOMAINS=' -e '^FERRY_STORAGE_LAYOUT=' "$lenv" > "$work/l" && { cat "$work/l"; echo 'FERRY_PUBLIC_URL=http://localhost:4290'; } > "$lenv"
awk '{ print } /^    environment:$/ && !done { print "      FERRY_PUBLIC_URL: ${FERRY_PUBLIC_URL:?Run scripts/quickstart.sh first}"; done = 1 }' "$repo/compose.production.yaml" > "$work/legacy/compose.production.yaml"
lsecrets=$(secrets_of "$lenv")
: > "$work/docker.log"
run --dir legacy >/dev/null
check 'grep -q "ferry-data:/data" "$work/legacy/compose.storage.yaml" && grep -q "postgres-data:/var/lib/postgresql/data" "$work/legacy/compose.storage.yaml" && ! grep -q "^FERRY_STORAGE_LAYOUT=directory" "$lenv"' 'an unmarked existing installation retains named volumes instead of opening an empty database'
check 'has "$lenv" "FERRY_PUBLIC_URL=http://localhost:4290" && [ "$lsecrets" = "$(secrets_of "$lenv")" ] && ! grep -q -- "--profile redis up" "$work/docker.log"' 'an earlier deployment keeps the public URL its Compose file requires and keeps Redis off without a Redis container'
run --dir legacy --domain ops.example.com --init-only >/dev/null
check 'has "$lenv" "FERRY_PUBLIC_URL=https://ops.example.com" && has "$lenv" "FERRY_DOMAINS=ops.example.com" && [ -f "$work/legacy/compose.https.yaml" ] && [ "$lsecrets" = "$(secrets_of "$lenv")" ]' 'an earlier deployment gets HTTPS without upgrading first'

# --upgrade on the Postgres deployment created first (v9.9.9).
image_line() { grep '^FERRY_IMAGE=' "$1"; }
without_image() { grep -v '^FERRY_IMAGE=' "$1"; }
refused --dir absent --upgrade
check '[ ! -e "$work/absent" ]' 'upgrade refuses a directory without a deployment and creates nothing'
refused --upgrade --port 1234
refused --upgrade --domain ops.example.com
: > "$work/docker.log"
output=$(run --upgrade)
check 'printf "%s" "$output" | grep -q "Already on v9.9.9" && grep -q "inspect --format" "$work/docker.log" && ! grep -qE " pull$| up -d " "$work/docker.log" && [ "$before" = "$(cat "$env")" ]' 'upgrade skips only an actually running healthy release, preserving configuration'
for older in v9.9.1 v9.9.9-rc.1; do refused --upgrade --version "$older"; done
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
check 'grep -q -- "--profile redis -f compose.storage.yaml pull" "$work/docker.log" && [ ! -e "$work/ferry/.upgrade" ] && printf "%s" "$output" | grep -q "Upgraded from v9.9.9 to v9.10.0" && printf "%s" "$output" | grep -q setup-token-123' 'keeps recorded Redis, leaves no staging files and prints the setup token'
upgraded=$(cat "$env")
if (export DOCKER_FAIL=pull; run --upgrade --version v9.11.0 >/dev/null 2>&1); then echo 'FAIL failed pull reported success'; exit 1; fi
check '[ "$upgraded" = "$(cat "$env")" ]' 'a failed image pull restores the previous files'
for signal in INT TERM; do
    if (export INTERRUPT_PULL=$signal; run --upgrade --version v9.11.0 >/dev/null 2>&1); then echo 'FAIL interrupted pull reported success'; exit 1; fi
    check '[ "$upgraded" = "$(cat "$env")" ]' "$signal during pull restores the previous configuration before any startup"
done
if (export DOCKER_FAIL='up -d'; run --upgrade --version v10.0.1 > /dev/null 2> "$work/err"); then echo 'FAIL unhealthy upgrade reported success'; exit 1; fi
check 'grep -q "did not become healthy" "$work/err" && grep -q "psql -U postgres -d ferry" "$work/err" && grep -q "v9.10.0-to-v10.0.1/database.sql" "$work/err"' 'an unhealthy release prints the rollback commands with its backup'
: > "$work/docker.log"
output=$(run --upgrade --version v10.0.1)
check 'printf "%s" "$output" | grep -q "Resuming deployment" && printf "%s" "$output" | grep -q "Deployment resumed successfully" && grep -q " pull$" "$work/docker.log" && grep -q " up -d " "$work/docker.log" && has "$work/ferry/.test-running-image" "ghcr.io/7link-team/ferry-server:v10.0.1"' 'interrupted upgrade with the target already recorded resumes from the older running container'
: > "$work/docker.log"
(export RUNNING_HEALTH=unhealthy; run --upgrade --version v10.0.1 >/dev/null 2>"$work/err") && { echo 'FAIL unhealthy resumed container reported success'; exit 1; }
check 'grep -q "not reached the healthy" "$work/err" && grep -q " up -d " "$work/docker.log"' 'matching image with failed health is reconciled and never reported as already upgraded'
rm "$work/ferry/.test-running-image"
: > "$work/docker.log"
output=$(run --upgrade --version v10.0.1)
check 'printf "%s" "$output" | grep -q "Deployment resumed successfully" && grep -q " up -d " "$work/docker.log"' 'missing Ferry container is recreated even when the configured version matches'
printf '%s\n' 'ghcr.io/7link-team/ferry-server:v11.0.0' > "$work/ferry/.test-running-image"
: > "$work/docker.log"
run --upgrade --version v10.0.1 >/dev/null 2>"$work/err" && { echo 'FAIL resume downgraded a newer running release'; exit 1; }
check 'grep -q "refusing to replace running" "$work/err" && ! grep -qE " pull$| up -d " "$work/docker.log"' 'resume refuses an actual runtime downgrade even if the configuration is stale'
: > "$work/docker.log"
run --dir legacy --upgrade --version v9.10.0 >/dev/null
check '! grep -q -- "--profile redis -f compose.storage.yaml pull" "$work/docker.log" && grep -q -- "-f compose.https.yaml -f compose.storage.yaml pull" "$work/docker.log" && ! grep -q PUBLIC_URL "$lenv" && grep -qx "ops.example.com {" "$work/legacy/caddy/Caddyfile"' 'upgrading keeps Redis off and the HTTPS proxy on, and drops the public URL the new files no longer need'
: > "$work/docker.log"
(export REDIS_UP=1; run --dir legacy --upgrade --version v10.0.0 >/dev/null)
check 'grep -qE -- "--profile redis( -f compose.https.yaml)? -f compose.storage.yaml pull" "$work/docker.log"' 'an earlier deployment with a Redis container keeps Redis'
: > "$work/docker.log"
run --dir mysql --upgrade --version v9.10.0 >/dev/null
check 'grep -q "exec -T mysql sh -c MYSQL_PWD=.* mysqldump" "$work/docker.log" && grep -q "compose.mysql.yaml --profile redis -f compose.storage.yaml pull" "$work/docker.log" && has "$work/mysql/.env.production" "FERRY_DATABASE_ENGINE=mysql"' 'a MySQL deployment is backed up with mysqldump and upgraded in place'
(cd "$work" && sh -s -- --dir mysql --upgrade --version v1.0.0 < "$repo/scripts/quickstart.sh" >/dev/null 2>&1) && { echo 'FAIL piped downgrade accepted'; exit 1; }
run --dir semver --version v1.0.0-alpha.9 --init-only >/dev/null
run --dir semver --upgrade --version v1.0.0-alpha.10 >/dev/null
run --dir semver --upgrade --version v1.0.0 >/dev/null
check 'image_line "$work/semver/.env.production" | grep -q ":v1.0.0$"' 'orders pre-releases numerically (alpha.9 < alpha.10 < 1.0.0)'
(cd "$work" && sh -s -- --dir piped --version v9.9.7 --init-only < "$repo/scripts/quickstart.sh" >/dev/null)
check 'has "$work/piped/.env.production" "FERRY_IMAGE=ghcr.io/7link-team/ferry-server:v9.9.7"' 'works when piped to sh with arguments (curl ... | sh -s -- ...)'
half=$(($(wc -l < "$repo/scripts/quickstart.sh") / 2))
(cd "$work" && head -n "$half" "$repo/scripts/quickstart.sh" | sh -s -- --dir truncated --version v9.9.7 --init-only >/dev/null 2>&1) || true
check '[ ! -e "$work/truncated" ]' 'a truncated download executes nothing'
mkdir -p "$work/orphaned/data/ferry"
echo retained > "$work/orphaned/data/ferry/fixture"
refused --dir orphaned --init-only
check '[ ! -e "$work/orphaned/.env.production" ] && [ "$(cat "$work/orphaned/data/ferry/fixture")" = retained ]' 'missing configuration never regenerates keys over existing directory data'
output=$(run --migrate-storage)
check 'printf "%s" "$output" | grep -q "already uses"' 'directory migration is idempotent'

echo 'quickstart verified'
