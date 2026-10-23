#!/usr/bin/env bash
# Exercises crates/server/src/ssh.rs against a real sshd in Docker.
#
# The SSH channel is almost entirely I/O against another program, so unit tests
# can only cover the fingerprint comparison and the output cap. Everything that
# actually broke while writing it — a pty that never reports its size, a
# download whose stream outlives the connection that feeds it, an upload that
# reports success before the last write is acknowledged — only shows up against
# a real server, so this script is the gate.
#
#   scripts/test-ssh.sh
#
# Phase 1 runs the whole live suite against a fresh container and cross-checks
# the fingerprint our code learned against ssh-keygen's own. Phase 2 throws that
# container away and starts another one, which has a different host key, then
# proves the connection is refused when the recorded fingerprint no longer
# matches. Two containers is the only honest way to test that: a fabricated
# fingerprint proves the comparison, not the scenario.
set -uo pipefail

cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"

IMAGE=ferry-ssh-test
NAME=ferry-ssh-test
PORT=${FERRY_TEST_SSH_PORT:-22022}
USER_NAME=ferry
PASSWORD=ferry-test

command -v docker >/dev/null || { echo "docker not found"; exit 1; }
trap 'docker rm -f "$NAME" >/dev/null 2>&1' EXIT

# The host key is generated at container start, not baked into the image, so
# every container is a different machine as far as TOFU is concerned.
docker build -q -t "$IMAGE" -f - . >/dev/null <<'DOCKERFILE' || exit 1
FROM alpine:3.21
RUN apk add --no-cache openssh-server openssh-sftp-server coreutils \
 && adduser -D -s /bin/sh ferry \
 && echo 'ferry:ferry-test' | chpasswd \
 && printf '%s\n' \
      'HostKey /etc/ssh/ssh_host_ed25519_key' \
      'PasswordAuthentication yes' \
      'PermitRootLogin no' \
      'Subsystem sftp /usr/lib/ssh/sftp-server' \
    > /etc/ssh/sshd_config
EXPOSE 22
CMD ["sh", "-c", "ssh-keygen -q -t ed25519 -N '' -f /etc/ssh/ssh_host_ed25519_key && exec /usr/sbin/sshd -D -e"]
DOCKERFILE

start_sshd() {
  docker rm -f "$NAME" >/dev/null 2>&1
  docker run -d --name "$NAME" -p "127.0.0.1:$PORT:22" "$IMAGE" >/dev/null || return 1
  local i
  for i in $(seq 1 60); do
    if (exec 3<>"/dev/tcp/127.0.0.1/$PORT") 2>/dev/null; then
      exec 3>&-
      return 0
    fi
    sleep 0.5
  done
  echo "sshd never came up on $PORT"
  docker logs "$NAME" 2>&1 | tail -20
  return 1
}

# ssh-keygen's own idea of the fingerprint, so the test is not just comparing
# our implementation against itself.
host_fingerprint() {
  docker exec "$NAME" ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub | awk '{print $2}'
}

# --test-threads=1 keeps the resident-memory check in the large-file test
# meaningful: another test's buffers would show up as our growth.
live_tests() { # live_tests <filter> <minimum-tests>
  local log
  log=$(mktemp)
  FERRY_REQUIRE_LIVE=1 \
  FERRY_TEST_SSH_ADDR="127.0.0.1:$PORT" \
  FERRY_TEST_SSH_USER="$USER_NAME" \
  FERRY_TEST_SSH_PASSWORD="$PASSWORD" \
    cargo test -p ferry-server "$1" -- --test-threads=1 --nocapture 2>&1 | tee "$log"
  local status=${PIPESTATUS[0]}
  # A filter that matches nothing exits 0 and prints "running 0 tests", so the
  # script would announce success having run none. That is exactly how this
  # suite silently stopped covering anything once `mod ssh;` went missing.
  local ran
  ran=$(grep -oE '^running [0-9]+ tests?' "$log" | grep -oE '[0-9]+' | paste -sd+ - | bc)
  ran=${ran:-0}
  rm -f "$log"
  if [ "$status" -ne 0 ]; then
    echo "FAILED: cargo test exited $status"
    return 1
  fi
  if [ "$ran" -lt "$2" ]; then
    echo "FAILED: expected at least $2 test(s) for filter '$1' but $ran ran."
    echo "        Is `mod ssh;` declared in crates/server/src/main.rs?"
    return 1
  fi
  echo "   ($ran test(s) ran)"
}

echo "== phase 1: full suite against a fresh sshd"
start_sshd || exit 1
FIRST_KEY=$(host_fingerprint)
echo "   host key: $FIRST_KEY"
FERRY_TEST_SSH_EXPECT_KEY="$FIRST_KEY" live_tests ssh:: 11 || exit 1

echo "== phase 2: the same address, a different host key"
start_sshd || exit 1
SECOND_KEY=$(host_fingerprint)
echo "   host key: $SECOND_KEY"
[ "$FIRST_KEY" != "$SECOND_KEY" ] || { echo "both containers share a host key; nothing was tested"; exit 1; }
FERRY_TEST_SSH_PRIOR_KEY="$FIRST_KEY" live_tests ssh::tests::live_a_changed_host_key_is_refused 1 || exit 1

echo "All SSH checks passed."
