#!/bin/sh
# One-command single-host Docker Compose deployment:
#   curl -fsSL https://raw.githubusercontent.com/7link-team/ferry/main/scripts/quickstart.sh | sh
#   curl -fsSL .../quickstart.sh | sh -s -- --domain ops.example.com --domain ops.example.org
#   curl -fsSL .../quickstart.sh | sh -s -- --upgrade
# Creates (or reuses) a deployment directory with the released Compose files of
# one version and random credentials, then starts the stack and prints the
# first-time setup token. Re-running reuses every secret; access options given
# again (--domain, --proxy-domain, --allow-host, --bind, --port) replace the recorded ones.
# --upgrade backs up the database and configuration, then moves an existing
# deployment to a newer release while keeping every secret and address.
set -eu

# Everything runs from main, invoked on the last line: when piped from curl, a
# truncated download defines nothing runnable instead of executing part of it.
main() {
# Option values are split on spaces below, never glob-expanded.
set -f
repository=${FERRY_REPOSITORY:-7link-team/ferry}
image_repository=${FERRY_IMAGE_REPOSITORY:-ghcr.io/7link-team/ferry-server}
version=
database=
domains=
proxy_domains=
hosts=
bind=
port=
redis=
init_only=
upgrade=
migrate_storage=
configured=
directory=ferry

fail() { printf 'Quickstart failed: %s\n' "$*" >&2; exit 1; }
usage() {
    cat <<'USAGE'
Usage: quickstart.sh [options]
  --dir DIR             Deployment directory (default: ./ferry)
  --version vX.Y.Z      Release to deploy (default: latest published release)
  --database NAME       postgres (default) or mysql; fixed at first start
  --domain NAME         Serve HTTPS on NAME with an automatic, auto-renewed certificate;
                        repeat for several domains (needs DNS to this host and ports 80/443)
  --proxy-domain NAME   Use external HTTPS (e.g. cloudflared); allow NAME with secure
                        cookies, without starting Caddy; repeatable
  --allow-host NAME     Allow plain-HTTP access by NAME or IP, e.g. a LAN address with
                        --bind 0.0.0.0; repeatable; cannot be combined with --domain
  --bind ADDRESS        Published Ferry listener address (default: 0.0.0.0)
  --port PORT           Published Ferry port (default: 4290)
  --without-redis       Do not start the optional Redis service
  --init-only           Create configuration without running Docker
  --upgrade             Back up, then upgrade an existing deployment to the latest
                        release (or --version); secrets and addresses are kept
  --migrate-storage     Stop an existing stack, back up and copy named volumes into
                        ./data, verify the copy, then restart; keeps the old volumes
On an existing deployment, --domain/--proxy-domain/--allow-host replace the access names and
--bind/--port replace the listener; secrets are never changed.
USAGE
}

value() { [ "$#" -ge 2 ] && [ -n "$2" ] || fail "missing value for $1"; }
# Names are collected into space-separated lists, so allow only name characters.
name() { value "$@"; case "$2" in *[!A-Za-z0-9.:-]*) fail "$1 must be a bare host name (got $2)" ;; esac; }
while [ "$#" -gt 0 ]; do
    case "$1" in
        --dir) value "$@"; directory=$2; shift 2 ;;
        --version) value "$@"; version=$2; shift 2 ;;
        --database) value "$@"; database=$2; configured=$1; shift 2 ;;
        --domain) name "$@"; domains="$domains $(printf '%s' "$2" | tr 'A-Z' 'a-z')"; configured=$1; shift 2 ;;
        --proxy-domain) name "$@"; proxy_domains="$proxy_domains $(printf '%s' "$2" | tr 'A-Z' 'a-z')"; configured=$1; shift 2 ;;
        --allow-host) name "$@"; hosts="$hosts $(printf '%s' "$2" | tr 'A-Z' 'a-z')"; configured=$1; shift 2 ;;
        --bind) value "$@"; bind=$2; configured=$1; shift 2 ;;
        --port) value "$@"; port=$2; configured=$1; shift 2 ;;
        --without-redis) redis=false; configured=$1; shift ;;
        --init-only) init_only=1; configured=$1; shift ;;
        --upgrade) upgrade=1; shift ;;
        --migrate-storage) migrate_storage=1; shift ;;
        -h|--help) usage; exit 0 ;;
        *) usage >&2; fail "unknown option: $1" ;;
    esac
done

[ -z "$migrate_storage" ] || [ -z "$upgrade$configured$version" ] \
    || fail "--migrate-storage only accepts --dir; run upgrades and access changes separately"

# Configuring an external proxy uses the normal public listener unless overridden.
if [ -n "$proxy_domains" ] && [ -z "$bind" ]; then bind=0.0.0.0; fi

case "$database" in ''|postgres|mysql) ;; *) fail "--database must be postgres or mysql" ;; esac
if [ -n "$port" ]; then
    case "$port" in *[!0-9]*) fail "--port must be a number" ;; esac
    [ "${#port}" -le 5 ] && [ "$port" -ge 1 ] && [ "$port" -le 65535 ] || fail "--port must be between 1 and 65535"
fi
[ -z "$bind" ] || printf '%s\n' "$bind" | grep -Eq '^[0-9A-Fa-f:.]+$' || fail "--bind must be an IP address"
label='[a-z0-9]([a-z0-9-]*[a-z0-9])?'
for name in $domains $proxy_domains; do
    # A DNS name with a letter-led top-level label: certificates are issued for names, not IPs.
    printf '%s\n' "$name" | grep -Eq "^($label\.)+[a-z]([a-z0-9-]*[a-z0-9])?\$" \
        || fail "--domain/--proxy-domain must be a domain name such as ops.example.com (got $name)"
done
for name in $hosts; do
    printf '%s\n' "$name" | grep -Eq "^($label\.)*$label\$|^[0-9a-f]*:[0-9a-f:.]*\$" \
        || fail "--allow-host must be a host name or IP address without scheme or port (got $name)"
done
[ -z "$proxy_domains" ] || [ -z "$domains$hosts" ] \
    || fail "--proxy-domain cannot be combined with --domain or --allow-host"
[ -z "$domains" ] || [ -z "$hosts" ] \
    || fail "--domain and --allow-host cannot be combined: HTTPS sign-in cookies are not sent over plain HTTP"
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
resolve_latest() {
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
}
download() { # DIRECTORY FILE...: each file of $version into DIRECTORY, or fail
    target=$1; shift
    for file in "$@"; do
        printf 'Downloading %s (%s)...\n' "$file" "$version"
        # Callers use `|| download`, where set -e is off: check every step.
        # curl can report success without writing a file for an empty reply.
        { fetch "https://raw.githubusercontent.com/$repository/$version/$file" "$target/$file.download" \
            && [ -s "$target/$file.download" ] \
            && mv "$target/$file.download" "$target/$file"; } \
            || { rm -f "$target/$file.download"; fail "cannot download $file for $version"; }
    done
}
# Succeeds when release $1 is newer than $2 by semantic versioning, so
# v1.0.0-alpha.10 > v1.0.0-alpha.9 and v1.0.0 > v1.0.0-rc.1.
newer() {
    awk -v a="${1#v}" -v b="${2#v}" '
    function ids(x, y,   xs, ys, nx, ny, i, p, q, pn, qn) {
        nx = split(x, xs, "."); ny = split(y, ys, ".")
        for (i = 1; i <= nx || i <= ny; i++) {
            if (i > nx) return -1
            if (i > ny) return 1
            p = xs[i]; q = ys[i]; pn = p ~ /^[0-9]+$/; qn = q ~ /^[0-9]+$/
            if (pn && qn) { if (p + 0 != q + 0) return p + 0 < q + 0 ? -1 : 1 }
            else if (pn != qn) return pn ? -1 : 1
            else if (p != q) return p < q ? -1 : 1
        }
        return 0
    }
    function release(x, y,   i, cx, cy, px, py, c) {
        sub(/\+.*/, "", x); sub(/\+.*/, "", y)
        i = index(x, "-"); cx = i ? substr(x, 1, i - 1) : x; px = i ? substr(x, i + 1) : ""
        i = index(y, "-"); cy = i ? substr(y, 1, i - 1) : y; py = i ? substr(y, i + 1) : ""
        c = ids(cx, cy)
        if (c != 0 || px == py) return c
        if (px == "") return 1
        if (py == "") return -1
        return ids(px, py)
    }
    BEGIN { exit !(release(a, b) > 0) }'
}
require_docker() {
    command -v docker >/dev/null 2>&1 && docker compose version >/dev/null 2>&1 \
        || fail "Docker with Compose v2 is required. Configuration is ready in $(pwd); install Docker and rerun this command."
}

umask 077
env_file=.env.production
files="compose.production.yaml compose.postgres.yaml compose.mysql.yaml scripts/postgres-init.sh"

env_get() { sed -n "s/^$1=//p" "$env_file" | head -n 1; }
env_set() { # KEY VALUE: replace or append one line, keeping everything else
    if grep -q "^$1=" "$env_file"; then
        awk -v key="$1" -v value="$2" 'index($0, key "=") == 1 { print key "=" value; next } { print }' "$env_file" > "$env_file.new"
    else
        { cat "$env_file"; printf '%s=%s\n' "$1" "$2"; } > "$env_file.new"
    fi
    mv "$env_file.new" "$env_file"
}
# Sets $compose for this deployment: its database, and the Redis and HTTPS
# services exactly as configured.
compose_for() {
    compose="docker compose --env-file $env_file -f compose.production.yaml -f compose.$(env_get FERRY_DATABASE_ENGINE).yaml"
    use_redis=$(env_get FERRY_REDIS)
    if [ -z "$use_redis" ]; then
        # Deployments created before FERRY_REDIS: keep Redis when its container exists.
        use_redis=false
        [ -z "$($compose --profile redis ps -a -q redis 2>/dev/null)" ] || use_redis=true
    fi
    [ "$use_redis" != true ] || compose="$compose --profile redis"
    [ ! -f compose.https.yaml ] || compose="$compose -f compose.https.yaml"
    [ ! -f compose.storage.yaml ] || compose="$compose -f compose.storage.yaml"
}
# Always override storage by container target. This also works with older release
# files, whose base mounts still name Docker volumes. No marker means legacy:
# downloading a new release MUST NOT silently switch an existing database.
storage_files() {
    layout=${1:-$(env_get FERRY_STORAGE_LAYOUT)}
    layout=${layout:-volume}
    case "$layout" in directory|volume) ;; *) fail "unknown FERRY_STORAGE_LAYOUT: $layout" ;; esac
    {
        printf '# Generated by quickstart; keep this file last in Compose commands.\nservices:\n'
        for service in ferry nats redis "$(env_get FERRY_DATABASE_ENGINE)"; do
            case "$service" in postgres) destination=/var/lib/postgresql/data ;; mysql) destination=/var/lib/mysql ;; *) destination=/data ;; esac
            if [ "$layout" = directory ]; then source=./data/$service; else source=$service-data; fi
            printf '  %s:\n    volumes: ["%s:%s"]\n' "$service" "$source" "$destination"
        done
        if [ -f compose.https.yaml ]; then
            if [ "$layout" = directory ]; then prefix=./data/; else prefix=; fi
            printf '  caddy:\n    volumes: ["%scaddy-data:/data", "%scaddy-config:/config"]\n' "$prefix" "$prefix"
        fi
        if [ "$layout" = volume ]; then
            printf 'volumes:\n'
            for service in ferry nats redis "$(env_get FERRY_DATABASE_ENGINE)"; do printf '  %s-data:\n' "$service"; done
            [ ! -f compose.https.yaml ] || printf '  caddy-data:\n  caddy-config:\n'
        fi
    } > compose.storage.yaml.new
    mv compose.storage.yaml.new compose.storage.yaml
}

# Official Ferry runs as 10001; NATS may run as 1000. Initialize only NEW empty
# directories, never recursively chown an existing database. DB/Redis official
# entrypoints perform their own ownership setup. The helper has no network.
prepare_storage() {
    [ "$(env_get FERRY_STORAGE_LAYOUT)" = directory ] || return 0
    mkdir -p data
    docker run --rm --network none --mount "type=bind,src=$(pwd -P)/data,dst=/target" \
        alpine:3.22 sh -ec '
        for name in ferry nats redis postgres mysql caddy-data caddy-config; do
            if [ ! -e "/target/$name" ]; then
                mkdir -m 700 "/target/$name"
                case "$name" in ferry) chown 10001:10001 "/target/$name" ;; nats) chown 1000:1000 "/target/$name" ;; esac
            fi
        done'
}
addresses() {
    names=$(env_get FERRY_DOMAINS | tr -d ',')
    if [ -n "$names" ]; then
        for name in $names; do printf '  https://%s\n' "$name"; done
    elif [ "$(env_get FERRY_SECURE_COOKIE)" = true ]; then
        for name in $(env_get FERRY_ALLOWED_HOSTS | tr ',' ' '); do
            case "$name" in localhost|127.0.0.1) ;; *) printf '  https://%s\n' "$name" ;; esac
        done
    else
        for name in $(env_get FERRY_ALLOWED_HOSTS | tr ',' ' '); do
            case "$name" in localhost|127.0.0.1) ;; *:*) printf '  http://[%s]:%s\n' "$name" "$(env_get FERRY_PORT)" ;; *) printf '  http://%s:%s\n' "$name" "$(env_get FERRY_PORT)" ;; esac
        done
        printf '  http://localhost:%s (from this machine)\n' "$(env_get FERRY_PORT)"
    fi
}
# The HTTPS proxy is generated here instead of shipped with a release: it does
# not depend on the Ferry version, so every deployment can use it right away.
https_files() {
    sites=$(env_get FERRY_DOMAINS)
    if [ -z "$sites" ]; then
        if [ -f compose.https.yaml ] && [ -z "$init_only" ] && command -v docker >/dev/null 2>&1; then
            compose_for
            $compose rm -s -f caddy >/dev/null 2>&1 \
                || printf 'Warning: could not stop the HTTPS proxy; run: %s rm -s -f caddy\n' "$compose" >&2
        fi
        rm -f compose.https.yaml caddy/Caddyfile
        return 0
    fi
    mkdir -p caddy
    {
        printf '# Generated by scripts/quickstart.sh from FERRY_DOMAINS; rerun it with --domain instead of editing.\n'
        printf '# Caddy obtains and renews a certificate per domain and redirects HTTP to HTTPS.\n'
        printf '%s {\n\treverse_proxy ferry:4290\n}\n' "$sites"
    } > caddy/Caddyfile.new
    # Caddy reads its configuration at start only: a changed site list on a
    # running proxy needs a restart (written in place to keep the bind mount).
    if [ -f caddy/Caddyfile ] && ! cmp -s caddy/Caddyfile.new caddy/Caddyfile; then sites_changed=1; fi
    cat caddy/Caddyfile.new > caddy/Caddyfile
    rm -f caddy/Caddyfile.new
    cat > compose.https.yaml.new <<'YAML'
# Generated by scripts/quickstart.sh for FERRY_DOMAINS; rerun it instead of editing.
services:
  caddy:
    image: ${CADDY_IMAGE:-caddy:2.10-alpine}
    restart: unless-stopped
    init: true
    security_opt: ["no-new-privileges:true"]
    logging:
      driver: json-file
      options: { max-size: "10m", max-file: "3" }
    ports: ["80:80", "443:443", "443:443/udp"]
    volumes:
      - ./caddy/Caddyfile:/etc/caddy/Caddyfile:ro
      - ./data/caddy-data:/data
      - ./data/caddy-config:/config
    networks: [egress]
    depends_on:
      ferry: { condition: service_healthy }
YAML
    mv compose.https.yaml.new compose.https.yaml
}
# Ferry never reads FERRY_PUBLIC_URL; Compose files of earlier releases still
# require it to be set, so keep it only for those.
public_url_compat() {
    if grep -q 'FERRY_PUBLIC_URL' compose.production.yaml; then
        first=$(env_get FERRY_DOMAINS | cut -d, -f1)
        if [ -n "$first" ]; then env_set FERRY_PUBLIC_URL "https://$first"
        else env_set FERRY_PUBLIC_URL "http://localhost:$(env_get FERRY_PORT)"; fi
    elif grep -q '^FERRY_PUBLIC_URL=' "$env_file"; then
        grep -v '^FERRY_PUBLIC_URL=' "$env_file" > "$env_file.new"; mv "$env_file.new" "$env_file"
    fi
}
setup_token() {
    printf 'First-time setup token (only present before initialization):\n'
    $compose exec -T ferry sh -c 'if [ -f /data/setup-token ]; then cat /data/setup-token; echo; else echo "Already initialized."; fi'
}

if [ -n "$migrate_storage" ]; then
    [ -f "$directory/$env_file" ] || fail "no deployment in $directory"
    cd "$directory"
    require_docker
    case "$(env_get FERRY_STORAGE_LAYOUT)" in
        directory) printf 'Storage already uses %s/data; nothing to migrate.\n' "$(pwd -P)"; exit 0 ;;
        ''|volume) ;;
        *) fail "unknown FERRY_STORAGE_LAYOUT" ;;
    esac
    database=$(env_get FERRY_DATABASE_ENGINE)
    case "$database" in postgres|mysql) ;; *) fail "invalid database engine" ;; esac
    project=$(env_get COMPOSE_PROJECT_NAME)
    project=${project:-ferry}
    case "$project" in *[!a-z0-9_-]*|'') fail "unsupported Compose project name" ;; esac
    here=$(pwd -P)
    case "$here" in *,*) fail "deployment paths containing commas are not supported by Docker --mount" ;; esac
    mkdir .storage-migration.lock 2>/dev/null || fail "storage migration is already running; inspect .storage-migration.lock"
    stopped= switched= backup= copied=
    running=
    migration_cleanup() {
        result=$?
        trap - EXIT INT TERM
        set +e
        if [ "$result" -ne 0 ]; then
            if [ -z "$switched" ]; then
                if [ -n "$backup" ]; then
                    cp -p "$backup/$env_file" "$env_file"
                    if [ -f "$backup/compose.storage.yaml" ]; then cp -p "$backup/compose.storage.yaml" compose.storage.yaml
                    else rm -f compose.storage.yaml; fi
                    for item in $copied; do
                        [ ! -e "data/$item" ] || mv "data/$item" "$backup/partial-$item"
                    done
                fi
                compose_for
                if [ -n "$stopped" ] && [ -n "$running" ]; then
                    $compose --profile redis up -d --wait --wait-timeout 300 $running \
                        || printf 'ERROR: could not restart the original services; run: %s up -d --wait\n' "$compose" >&2
                fi
                printf 'Migration failed before cutover; original named volumes are retained. Backup: %s\n' "${backup:-not started}" >&2
            else
                printf 'Directory storage is active but restart failed. Do NOT restore stale volumes. Inspect: %s logs --tail 100\n' "$compose" >&2
            fi
        fi
        rm -f .storage-migration.lock/volumes.txt
        rmdir .storage-migration.lock
        exit "$result"
    }
    trap migration_cleanup EXIT
    trap 'exit 130' INT
    trap 'exit 143' TERM
    compose_for
    $compose config --quiet
    # Pull before downtime, including images needed to restart a stopped stack.
    docker pull alpine:3.22
    $compose pull
    [ ! -L data ] || fail "data/ must not be a symlink"
    mkdir -p data backups
    inventory=.storage-migration.lock/volumes.txt
    : > "$inventory"
    total=0
    for item in ferry nats redis postgres mysql caddy-data caddy-config; do
        case "$item" in postgres) destination=/var/lib/postgresql/data ;; mysql) destination=/var/lib/mysql ;; caddy-config) destination=/config ;; *) destination=/data ;; esac
        service=$item
        case "$item" in caddy-*) service=caddy ;; esac
        container=$($compose --profile redis ps -a -q "$service" 2>/dev/null || true)
        source=
        if [ -n "$container" ]; then
            owner=$(docker inspect --format '{{index .Config.Labels "com.docker.compose.project.working_dir"}}' "$container")
            [ "$owner" = "$here" ] || fail "$service container belongs to $owner, not $here"
            source=$(docker inspect --format "{{range .Mounts}}{{if eq .Destination \"$destination\"}}{{if eq .Type \"volume\"}}{{.Name}}{{end}}{{end}}{{end}}" "$container")
            [ -n "$source" ] || fail "$service:$destination is not a named volume; mixed/custom mounts require manual migration"
        else
            source=${project}_${item}
            case "$item" in caddy-*) ;; *) source=$source-data ;; esac
            if ! docker volume inspect "$source" >/dev/null 2>&1; then
                case "$item" in ferry|nats|"$database") fail "required volume $source does not exist" ;; *) continue ;; esac
            fi
        fi
        volume_project=$(docker volume inspect --format '{{index .Labels "com.docker.compose.project"}}' "$source")
        [ "$volume_project" = "$project" ] || fail "$source does not belong to Compose project $project"
        # Refuse shared volumes: stopping this stack cannot quiesce outside writers.
        for consumer in $(docker ps -a -q --filter "volume=$source"); do
            owner=$(docker inspect --format '{{index .Config.Labels "com.docker.compose.project.working_dir"}}' "$consumer")
            [ "$owner" = "$here" ] || fail "$source is also mounted by a container from $owner"
        done
        [ ! -e "data/$item" ] && [ ! -L "data/$item" ] || fail "data/$item already exists; nothing will be overwritten"
        size=$(docker run --rm --network none --mount "type=volume,src=$source,dst=/source,readonly" alpine:3.22 \
            sh -ec 'find /source -type f -exec stat -c %s {} \; | awk "{n+=\$1} END {printf \"%.0f\\n\", n/1024+1}"')
        total=$((total + size))
        printf '%s %s\n' "$item" "$source" >> "$inventory"
    done
    # Archive plus extracted data; leave 100 MiB for filesystem metadata/startup.
    available=$(docker run --rm --network none --mount "type=bind,src=$here/data,dst=/target" alpine:3.22 sh -ec 'df -Pk /target | tail -1 | awk "{print \$4}"')
    [ "$available" -gt "$((total * 2 + 102400))" ] || fail "insufficient disk space for both archives and copied data"
    backup=backups/$(date -u +%Y%m%dT%H%M%SZ)-storage-$$
    mkdir "$backup"
    cp -p "$env_file" "$backup/$env_file"
    for file in compose.*.yaml; do cp -p "$file" "$backup/"; done
    [ ! -d caddy ] || cp -pR caddy "$backup/"
    cp "$inventory" "$backup/volumes.txt"
    rm -f "$inventory"
    running=$($compose --profile redis ps --services --status running)
    printf 'Stopping the stack before taking consistent volume archives...\n'
    stopped=1
    $compose --profile redis stop
    while read -r item source; do
        [ -z "$(docker ps -q --filter "volume=$source")" ] \
            || fail "$source still has a running writer; no data was copied"
    done < "$backup/volumes.txt"
    while read -r item source; do
        printf 'Backing up, copying and verifying %s...\n' "$item"
        copied="$copied $item"
        docker run --rm --network none \
            --mount "type=volume,src=$source,dst=/source,readonly" \
            --mount "type=bind,src=$here/data,dst=/target" \
            --mount "type=bind,src=$here/$backup,dst=/backup" \
            alpine:3.22 sh -ec '
                set -o pipefail
                name=$1
                tar -C /source -cpf "/backup/$name.tar" .
                (cd /backup && sha256sum "$name.tar" > "$name.tar.sha256")
                mkdir -m 700 "/target/$name"
                tar -C "/target/$name" -xpf "/backup/$name.tar"
                (cd /source && find . -type f -exec sha256sum {} \; | LC_ALL=C sort) > /backup/source-hashes
                (cd "/target/$name" && find . -type f -exec sha256sum {} \; | LC_ALL=C sort) > /backup/target-hashes
                cmp /backup/source-hashes /backup/target-hashes
                mv /backup/source-hashes "/backup/$name.files.sha256"
                rm /backup/target-hashes
                (cd /source && find . -exec stat -c "%N|%u|%g|%a|%Y|%F|%h" {} \; | LC_ALL=C sort) > /backup/source-metadata
                (cd "/target/$name" && find . -exec stat -c "%N|%u|%g|%a|%Y|%F|%h" {} \; | LC_ALL=C sort) > /backup/target-metadata
                cmp /backup/source-metadata /backup/target-metadata
                mv /backup/source-metadata "/backup/$name.metadata"
                rm /backup/target-metadata
            ' sh "$item"
    done < "$backup/volumes.txt"
    storage_files directory
    compose_for
    $compose config --quiet
    env_set FERRY_STORAGE_LAYOUT directory
    switched=1
    prepare_storage
    $compose up -d --wait --wait-timeout 300
    # Preserve a manually running optional Redis, even when its profile is off.
    for service in $running; do
        if [ "$service" = redis ] && [ "$use_redis" != true ]; then
            $compose --profile redis up -d --wait --wait-timeout 300 redis
        fi
    done
    printf 'Storage migration complete: %s/data. Archives: %s/%s. Original volumes were NOT deleted.\n' "$here" "$here" "$backup"
    exit 0
fi

if [ -n "$upgrade" ]; then
    [ ! -d "$directory/.storage-migration.lock" ] || fail "storage migration is in progress"
    [ -z "$configured" ] || fail "--upgrade keeps the existing configuration and only accepts --dir and --version (got $configured)"
    [ -f "$directory/$env_file" ] || fail "no deployment in $directory; run without --upgrade to create one"
    cd "$directory"
    current_image=$(env_get FERRY_IMAGE)
    current=${current_image##*:}
    image=${current_image%:*}
    database=$(env_get FERRY_DATABASE_ENGINE)
    case "$current_image" in *@*|*[!A-Za-z0-9._/:+-]*) fail "FERRY_IMAGE ($current_image) is not a release tag; set it to $image_repository:vX.Y.Z or upgrade manually" ;; esac
    case "$current" in v[0-9]*) ;; *) fail "cannot read the deployed release from FERRY_IMAGE ($current_image)" ;; esac
    case "$database" in postgres|mysql) ;; *) fail "FERRY_DATABASE_ENGINE in $env_file must be postgres or mysql" ;; esac
    [ -n "$version" ] || resolve_latest
    if [ "$version" = "$current" ]; then
        printf 'Already on %s; nothing to upgrade.\n' "$current"
        exit 0
    fi
    newer "$version" "$current" \
        || fail "$version is older than the deployed $current; downgrades are refused because database migrations only move forward"
    require_docker
    compose_for
    release="$files"

    backup=backups/$(date -u +%Y%m%dT%H%M%SZ)-$current-to-$version
    mkdir -p "$backup/scripts"
    printf 'Backing up the %s database to %s/%s/database.sql...\n' "$database" "$(pwd)" "$backup"
    if [ "$database" = postgres ]; then
        dump() { $compose exec -T postgres pg_dump -U postgres -d ferry --clean --if-exists; }
        load='exec -T postgres psql -U postgres -d ferry -v ON_ERROR_STOP=1'
    else
        dump() { $compose exec -T mysql sh -c 'MYSQL_PWD="$MYSQL_ROOT_PASSWORD" exec mysqldump -uroot --single-transaction --routines --triggers --databases ferry'; }
        load="exec -T mysql sh -c 'MYSQL_PWD=\"\$MYSQL_ROOT_PASSWORD\" exec mysql -uroot'"
    fi
    if ! dump > "$backup/database.sql" || [ ! -s "$backup/database.sql" ]; then
        fail "database backup failed, nothing was changed; the $database service must be running ($compose up -d)"
    fi
    saved=
    for file in "$env_file" $release compose.storage.yaml; do
        if [ -f "$file" ]; then cp -p "$file" "$backup/$file"; saved="$saved $file"; fi
    done
    restore() { for file in $saved; do cp -p "$backup/$file" "$file"; done; }

    # Stage the whole release before touching the deployment.
    rm -rf .upgrade
    mkdir -p .upgrade/scripts
    download .upgrade $release
    sed "s#^FERRY_IMAGE=.*#FERRY_IMAGE=$image:$version#" "$env_file" > ".upgrade/$env_file"
    for file in $release "$env_file"; do mv ".upgrade/$file" "$file"; done
    rm -rf .upgrade
    chmod 755 scripts/postgres-init.sh
    public_url_compat
    https_files
    storage_files
    compose_for

    $compose config --quiet \
        || { restore; fail "the $version configuration is invalid for this deployment; previous files restored, services untouched"; }
    $compose pull \
        || { restore; fail "cannot pull the $version images; previous files restored, services untouched"; }
    prepare_storage
    if ! $compose up -d --wait --wait-timeout 300; then
        printf 'Quickstart failed: %s did not become healthy; inspect it with: %s logs ferry\n' "$version" "$compose" >&2
        printf 'To return to %s, run in %s:\n' "$current" "$(pwd)" >&2
        printf '  cp -p %s/%s %s/compose.*.yaml . && cp -p %s/scripts/postgres-init.sh scripts/\n' "$backup" "$env_file" "$backup" "$backup" >&2
        printf '  %s stop ferry\n  %s %s < %s/database.sql\n  %s up -d --wait\n' "$compose" "$compose" "$load" "$backup" "$compose" >&2
        exit 1
    fi
    printf 'Upgraded from %s to %s. Backup kept in %s/%s.\nOpen Ferry at:\n' "$current" "$version" "$(pwd)" "$backup"
    addresses
    setup_token
    exit 0
fi

[ ! -d "$directory/.storage-migration.lock" ] || fail "storage migration is in progress"
mkdir -p "$directory/scripts"
cd "$directory"

if [ ! -f "$env_file" ] && [ -d data ] && [ -n "$(ls -A data)" ]; then
    fail "data/ already contains files but $env_file is missing; restore the original configuration and master key"
fi

if [ -f "$env_file" ]; then
    deployed=$(env_get FERRY_DATABASE_ENGINE)
    [ -z "$database" ] || [ "$database" = "$deployed" ] \
        || fail "this deployment uses $deployed; switching databases requires a separate directory and a data migration"
    database=$deployed
fi
[ -n "$database" ] || database=postgres

# Every deployment uses the Compose project "ferry": a second directory would
# take over the first one's containers and data volumes with new credentials.
# Only creating a deployment is refused; an existing one may always take its
# containers back (rerun or --upgrade in its own directory).
if [ ! -f "$env_file" ] && command -v docker >/dev/null 2>&1; then
    here=$(pwd -P)
    owner=$(docker ps -a --filter label=com.docker.compose.project=ferry --format '{{.Label "com.docker.compose.project.working_dir"}}' 2>/dev/null \
        | grep -vxF "$here" | head -n 1 || true)
    [ -z "$owner" ] || fail "a Ferry deployment already runs from $owner; rerun this command in $(dirname "$owner") (or pass --dir $owner) instead of creating a second deployment in $here"
    if [ -z "$(docker ps -a -q --filter label=com.docker.compose.project=ferry 2>/dev/null)" ] \
        && [ -n "$(docker volume ls -q --filter label=com.docker.compose.project=ferry 2>/dev/null)" ]; then
        fail "data volumes of an earlier Ferry deployment exist (docker volume ls --filter label=com.docker.compose.project=ferry); run this command in the directory holding that deployment's .env.production, or remove those volumes deliberately"
    fi
fi

missing=
for file in $files; do [ -f "$file" ] || missing=1; done
if [ -n "$missing" ] || [ ! -f "$env_file" ]; then
    if [ -z "$version" ] && [ -f "$env_file" ]; then
        version=$(sed -n 's/^FERRY_IMAGE=.*:\(v[^:@]*\)$/\1/p' "$env_file" | head -n 1)
    fi
    [ -n "$version" ] || resolve_latest
    for file in $files; do [ -f "$file" ] || download . "$file"; done
    chmod 755 scripts/postgres-init.sh
fi

if [ ! -f "$env_file" ]; then
    # Hex secrets are safe inside SQL and NATS URLs without encoding.
    {
        printf '# Keep this file and data/ together. Never regenerate the master key.\n'
        printf 'FERRY_DATABASE_ENGINE=%s\n' "$database"
        printf 'FERRY_STORAGE_LAYOUT=directory\n'
        printf 'COMPOSE_PROJECT_NAME=ferry\n'
        printf 'FERRY_IMAGE=%s:%s\n' "$image_repository" "$version"
        printf 'FERRY_DOMAINS=\n'
        printf 'FERRY_ALLOWED_HOSTS=localhost,127.0.0.1\n'
        printf 'FERRY_SECURE_COOKIE=false\n'
        printf 'FERRY_BIND_ADDRESS=0.0.0.0\n'
        printf 'FERRY_PORT=4290\n'
        printf 'FERRY_REDIS=%s\n' "${redis:-true}"
        for key in FERRY_MASTER_KEY DATABASE_PASSWORD POSTGRES_ROOT_PASSWORD MYSQL_ROOT_PASSWORD NATS_PASSWORD REDIS_PASSWORD; do
            printf '%s=%s\n' "$key" "$(random)"
        done
    } > "$env_file.new"
    mv "$env_file.new" "$env_file"
    printf 'Created %s/%s with random credentials. Back up this file securely.\n' "$(pwd)" "$env_file"
elif [ -n "$domains$proxy_domains$hosts$bind$port$redis" ]; then
    printf 'Updating access settings in %s/%s; secrets are unchanged.\n' "$(pwd)" "$env_file"
else
    printf 'Reusing %s/%s unchanged.\n' "$(pwd)" "$env_file"
fi

# Requested access settings replace the recorded ones; secrets never change.
if [ -n "$domains$proxy_domains$hosts" ]; then
    names=$(printf '%s\n' $domains $proxy_domains $hosts | awk '!seen[$0]++' | tr '\n' ',')
    env_set FERRY_DOMAINS "$(printf '%s\n' $domains | awk '!seen[$0]++' | sed 's/$/, /' | tr -d '\n' | sed 's/, $//')"
    env_set FERRY_ALLOWED_HOSTS "${names}localhost,127.0.0.1"
    if [ -n "$domains$proxy_domains" ]; then env_set FERRY_SECURE_COOKIE true; else env_set FERRY_SECURE_COOKIE false; fi
fi
[ -z "$bind" ] || env_set FERRY_BIND_ADDRESS "$bind"
[ -z "$port" ] || env_set FERRY_PORT "$port"
[ -z "$redis" ] || env_set FERRY_REDIS "$redis"
public_url_compat
https_files
storage_files

[ -z "$init_only" ] || exit 0
require_docker
prepare_storage
compose_for
$compose config --quiet
$compose up -d --wait --wait-timeout 300
[ -z "${sites_changed:-}" ] || $compose restart caddy
printf 'Stack is healthy. Open Ferry at:\n'
addresses
if [ -n "$(env_get FERRY_DOMAINS)" ]; then
    printf 'Caddy requests the certificates now and renews them automatically. Each domain must resolve to this\n'
    printf 'server and ports 80 and 443 must be reachable; on errors run: %s logs caddy\n' "$compose"
fi
setup_token
}

main "$@"
