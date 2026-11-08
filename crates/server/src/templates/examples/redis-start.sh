#!/bin/sh
set -eu
umask 077

ipv4() {
  case "$1" in ''|*[!0-9.]*|127.*|0.*) return 1 ;; esac
  old_ifs=$IFS
  IFS=.
  set -f
  set -- $1
  IFS=$old_ifs
  [ "$#" -eq 4 ] || return 1
  for octet do
    [ -n "$octet" ] && [ "$octet" -le 255 ] || return 1
  done
}
ipv4 "$REDIS_NODE_IP" && ipv4 "$REDIS_INITIAL_IP" || {
  printf '%s\n' 'Redis requires reachable non-loopback IPv4 addresses.' >&2
  exit 1
}
: "${REDISCLI_AUTH:?A deploy secret variable is required}"
# Redis 7.4 Sentinel rewrites auth-pass values without quoting. Multi-line
# credentials cannot be safely recovered as single configuration directives.
case "$REDISCLI_AUTH" in
  *'
'*|*"$(printf '\r')"*) printf '%s\n' 'Redis Sentinel credentials must be a single line.' >&2; exit 1 ;;
esac
# Quote a Redis configuration literal inside a single native option argument.
# This also keeps passwords beginning with "--" from becoming server options.
auth=$(awk 'BEGIN {
  s=ENVIRON["REDISCLI_AUTH"]; printf "\""
  for (i=1; i<=length(s); i++) {
    c=substr(s,i,1)
    if (c=="\\" || c=="\"") printf "\\%s",c
    else if (c=="\n") printf "\\n"
    else if (c=="\r") printf "\\r"
    else if (c=="\t") printf "\\t"
    else printf "%s",c
  }
  printf "\""
}')
mkdir -p /data

case "$REDIS_KIND" in
  data)
    config=/data/redis.conf
    if [ ! -e "$config" ]; then
      draft=$(mktemp /data/redis.conf.XXXXXX)
      {
        printf 'bind 0.0.0.0\nport %s\ndir /data\nappendonly yes\nappendfsync everysec\nsave ""\nmin-replicas-to-write 1\nmin-replicas-max-lag 10\n' "$REDIS_PORT"
        if [ "$REDIS_INITIAL_ROLE" = replica ]; then
          printf 'replicaof %s %s\n' "$REDIS_INITIAL_IP" "$REDIS_INITIAL_PORT"
        fi
      } > "$draft"
      mv "$draft" "$config"
    fi
    set -- redis-server "$config" --port "$REDIS_PORT" "--requirepass $auth" "--masterauth $auth" \
      --replica-announce-ip "$REDIS_NODE_IP" --replica-announce-port "$REDIS_PORT"
    ;;
  sentinel)
    config=/data/sentinel.conf
    if [ ! -e "$config" ]; then
      draft=$(mktemp /data/sentinel.conf.XXXXXX)
      printf 'bind 0.0.0.0\nport %s\ndir /data\nsentinel monitor example %s %s 2\nsentinel down-after-milliseconds example 5000\nsentinel failover-timeout example 60000\nsentinel parallel-syncs example 1\n' \
        "$REDIS_PORT" "$REDIS_INITIAL_IP" "$REDIS_INITIAL_PORT" > "$draft"
      mv "$draft" "$config"
    fi
    # Preserve elected master, peers, epochs and Sentinel identity. Only auth
    # is supplied again via safely quoted native arguments below: Redis 7.4
    # may have persisted these two directives with unescaped special chars.
    draft=$(mktemp /data/sentinel.conf.XXXXXX)
    awk '!($1 == "sentinel" && ($2 == "sentinel-pass" || ($2 == "auth-pass" && $3 == "example")))' "$config" > "$draft"
    mv "$draft" "$config"
    set -- redis-server "$config" --port "$REDIS_PORT" "--requirepass $auth" \
      "--sentinel auth-pass example $auth" "--sentinel sentinel-pass $auth" \
      '--sentinel announce-ip' "$REDIS_NODE_IP" '--sentinel announce-port' "$REDIS_PORT" --sentinel
    ;;
  *) printf '%s\n' 'Unknown Redis member kind.' >&2; exit 1 ;;
esac
# Reuse the pinned image's entrypoint to give the redis user its volume files
# and drop root privileges, including on the first boot of a fresh volume.
exec /usr/local/bin/docker-entrypoint.sh "$@"
