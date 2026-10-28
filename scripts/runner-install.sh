#!/bin/sh
# Download verified release bytes, then use the native service installer.
set -eu
fail() { printf '%s\n' "$*" >&2; exit 1; }
server=
name=runner-agent
while [ "$#" -gt 0 ]; do
    case "$1" in
        --server) [ "$#" -ge 2 ] || fail 'Missing --server value'; server=$2; shift 2 ;;
        --name) [ "$#" -ge 2 ] || fail 'Missing --name value'; name=$2; shift 2 ;;
        *) fail 'Usage: runner-install.sh --server URL [--name SERVICE]' ;;
    esac
done
case "$server" in http://?*|https://?*) ;; *) fail '--server must be an HTTP(S) URL' ;; esac
case "$server" in *'?'*|*'#'*|*'@'*|*[[:space:]]*) fail 'Invalid server URL' ;; esac
case "$name" in ''|[!a-zA-Z0-9]*|*[!a-zA-Z0-9._-]*) fail 'Invalid service name' ;; esac
[ "${#name}" -le 80 ] || fail 'Service name exceeds 80 characters'
[ -n "${FERRY_TOKEN:-}" ] || fail 'Set FERRY_TOKEN to the enrolment token first'
case "$FERRY_TOKEN" in *[!a-zA-Z0-9._~-]*) fail 'Invalid enrolment token' ;; esac
# Reconstruct only validated options; sudo receives no token in its argv.
if [ "$(id -u)" != 0 ]; then
    exec sudo --preserve-env=FERRY_TOKEN /bin/sh "$0" --server "$server" --name "$name"
fi
case "$(uname -s)/$(uname -m)" in
    Linux/x86_64) target=x86_64-unknown-linux-gnu ;;
    Linux/aarch64|Linux/arm64) target=aarch64-unknown-linux-gnu ;;
    Darwin/x86_64) target=x86_64-apple-darwin ;;
    Darwin/arm64|Darwin/aarch64) target=aarch64-apple-darwin ;;
    *) fail 'Unsupported operating system or CPU architecture' ;;
esac
command -v curl >/dev/null 2>&1 || fail 'curl is required'
umask 077
stage=$(mktemp -d /private/tmp/runner-install.XXXXXXXX 2>/dev/null || mktemp -d /tmp/runner-install.XXXXXXXX)
trap 'rm -rf "$stage"' EXIT
trap 'exit 1' HUP INT TERM
printf 'Authorization: Bearer %s\n' "$FERRY_TOKEN" > "$stage/auth"
# Do not redirect this authenticated request to another origin.
curl --fail --silent --show-error --connect-timeout 15 --max-time 60 \
    --max-filesize 8192 --proto '=http,https' --header "@$stage/auth" \
    --output "$stage/metadata" "${server%/}/api/runners/install/$target"
[ "$(wc -c < "$stage/metadata" | tr -d ' ')" -le 8192 ] || fail 'Installation metadata is too large'
# Exactly four newline-terminated lines; no shell evaluation of metadata.
[ "$(wc -l < "$stage/metadata" | tr -d ' ')" = 4 ] || fail 'Invalid installation metadata'
exec 3< "$stage/metadata"
IFS= read -r version <&3
IFS= read -r url <&3
IFS= read -r expected_hash <&3
IFS= read -r expected_size <&3
extra=
if IFS= read -r extra <&3 || [ -n "$extra" ]; then fail 'Invalid installation metadata'; fi
exec 3<&-
case "$version" in ''|*[!a-zA-Z0-9.+-]*) fail 'Invalid release version' ;; esac
case "$url" in https://?*) ;; *) fail 'Artifact URL must use HTTPS' ;; esac
case "$url" in *[[:space:]]*) fail 'Invalid artifact URL' ;; esac
[ "${#expected_hash}" = 64 ] || fail 'Invalid SHA256'
case "$expected_hash" in *[!0-9a-f]*) fail 'Invalid SHA256' ;; esac
case "$expected_size" in ''|0*|*[!0-9]*) fail 'Invalid artifact size' ;; esac
[ "${#expected_size}" -le 9 ] && [ "$expected_size" -le 268435456 ] || fail 'Artifact exceeds size limit'
curl --fail --silent --show-error --location --max-redirs 5 --connect-timeout 15 \
    --max-time 300 --max-filesize "$expected_size" --proto '=https' --proto-redir '=https' \
    --output "$stage/runner" "$url"
actual_size=$(wc -c < "$stage/runner" | tr -d ' ')
[ "$actual_size" = "$expected_size" ] || fail 'Artifact size mismatch; nothing installed'
if command -v sha256sum >/dev/null 2>&1; then
    actual_hash=$(sha256sum "$stage/runner")
else
    actual_hash=$(shasum -a 256 "$stage/runner")
fi
actual_hash=${actual_hash%% *}
[ "$actual_hash" = "$expected_hash" ] || fail 'Artifact SHA256 mismatch; nothing installed'
chmod 700 "$stage/runner"
printf 'Installing Runner %s as %s…\n' "$version" "$name"
"$stage/runner" service install --server "$server" --name "$name"
