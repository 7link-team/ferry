#!/usr/bin/env bash
# Drives a real ferry-runner against a real ferry-server and checks the things
# unit tests cannot: that enrolment is single-use, that the agent reconnects by
# itself, that a pty echoes, that a 100 MB+ file crosses in both directions
# with its hash intact and the server's memory flat, and that one workspace's
# runner cannot reach another workspace's host.
#
#   scripts/test-runner.sh
#   FERRY_TEST_SIZE_MB=500 scripts/test-runner.sh    # bigger transfer
#
# This script does not modify any source file. It needs these already wired in
# crates/server/src/main.rs, and says so rather than patching them in:
#
#   mod runner;
#   .route("/api/runners/connect", get(runner::connect))
#   .route("/api/runners/sessions/{session}", get(runner::session))
#   .route("/{workspace}/runner-tokens", post(runner::create_token))
#   "runner" => runner::open(s, host).await,     // in channel_open
#
# Every step is judged by an exit code or by a value read back out of the
# system. All checks and dependencies are required; nothing may be skipped.
set -uo pipefail

cd "$(dirname "$0")/.."
# These are mandatory: missing tooling must not look like an empty check.
for dependency in cargo curl python3 lsof shasum pkill ps; do
  command -v "$dependency" >/dev/null || { echo "FATAL missing dependency: $dependency"; exit 1; }
done
busy() { lsof -ti "TCP@127.0.0.1:$1" -sTCP:LISTEN >/dev/null 2>&1; }

# Pick a free port rather than insisting on one. Two agents share this
# repository and both run harnesses; a fixed default turns "someone else is
# testing right now" into a failure of this script.
if [ -n "${FERRY_TEST_PORT:-}" ]; then
  PORT=$FERRY_TEST_PORT
else
  PORT=
  for candidate in $(seq 4320 4360); do
    busy "$candidate" || { PORT=$candidate; break; }
  done
  [ -n "$PORT" ] || { echo "FATAL no free port in 4320-4360"; exit 1; }
fi
B="http://127.0.0.1:$PORT/api"
WORK="${TMPDIR:-/tmp}/ferry-runner-check.$$"
SIZE_MB=${FERRY_TEST_SIZE_MB:-120}
pass=0
fail=0

mkdir -p "$WORK"

cleanup() {
  [ -n "${SAMPLER:-}" ] && kill "$SAMPLER" 2>/dev/null
  [ -n "${RUNNER_PID:-}" ] && kill "$RUNNER_PID" 2>/dev/null
  # Every server this run started, not just the most recent: the reconnect
  # check starts a second one, and leaking it made the *next* run of this
  # script die on "something else is already serving on port".
  for pid in ${SERVER_PIDS:-}; do kill "$pid" 2>/dev/null; done
  # Backstop for a run that died before its pids were recorded: a leaked runner
  # does not hold the port, it reconnects to whatever appears on it next and
  # shows up as an extra online host in the following run. $WORK is unique to
  # this run, so matching on it cannot reach anyone else's process.
  pkill -f "state-dir $WORK" 2>/dev/null
  wait 2>/dev/null
  # Killing is asynchronous; the port is what the next run actually needs back.
  for _ in $(seq 1 40); do
    curl -sf -m 1 "$B/status" >/dev/null 2>&1 || break
    sleep 0.25
  done
  rm -rf "$WORK"
}
trap cleanup EXIT

ok()   { pass=$((pass + 1)); printf '  ok   %s\n' "$1"; }
bad()  { fail=$((fail + 1)); printf '  FAIL %s\n' "$1"; }


ck() { # ck <label> <expected-regex> <actual>
  if printf '%s' "$3" | grep -q "$2"; then
    ok "$1"
  else
    bad "$1: want /$2/ got: $(printf '%s' "$3" | head -c 300)"
  fi
}
ckn() { # ckn <label> <unwanted-regex> <actual>
  if printf '%s' "$3" | grep -q "$2"; then
    bad "$1: did not want /$2/ got: $(printf '%s' "$3" | head -c 300)"
  else
    ok "$1"
  fi
}
die() { printf 'FATAL %s\n' "$1"; exit 1; }
field() { python3 -I -c "import json,sys;d=json.load(sys.stdin);print(d.get(sys.argv[1],''))" "$1" 2>/dev/null; }

# ------------------------------------------------------------ build and unit tests

echo "== building"
# Judged by exit code, not by reading the output: a stale binary left over from
# an earlier build would otherwise sail straight past a broken compile.
if ! cargo build -p ferry-server -p ferry-runner >"$WORK/build.log" 2>&1; then
  tail -30 "$WORK/build.log"
  die "cargo build failed"
fi
SERVER=target/debug/ferry-server
RUNNER=target/debug/ferry-runner
[ -x "$SERVER" ] || die "no $SERVER"
[ -x "$RUNNER" ] || die "no $RUNNER"

echo "== unit tests"
cargo test --workspace >"$WORK/test.log" 2>&1
TEST_EXIT=$?
# A suite that matched nothing exits 0 and reports "0 passed", so the count is
# checked as well as the status: a gate that can pass vacuously is not a gate.
RUNNER_TESTS=$(grep -c '^test runner::tests::' "$WORK/test.log")
SESSION_TESTS=$(grep -c '^test session::tests::' "$WORK/test.log")
if [ "$TEST_EXIT" -ne 0 ]; then
  grep -E '^(test .* FAILED|failures:|test result: FAILED)' "$WORK/test.log" | head -20
  bad "cargo test --workspace exited $TEST_EXIT"
elif [ "$RUNNER_TESTS" -lt 1 ] || [ "$SESSION_TESTS" -lt 1 ]; then
  bad "cargo test ran no runner tests (runner::tests=$RUNNER_TESTS session::tests=$SESSION_TESTS) — the module is probably not declared"
else
  ok "cargo test --workspace (exit 0, $RUNNER_TESTS runner + $SESSION_TESTS session tests ran)"
fi

# ---------------------------------------------------------------------- server

DATA="$WORK/data"
SLOG="$WORK/server.log"
start_server() {
  # Wait for the port to be free, then refuse to proceed if something is still
  # answering on it. A port that replies is not proof that it replies for us:
  # a stale server with an old database answers /api/status perfectly well,
  # our own bind then fails silently, and every later check fails for a reason
  # that has nothing to do with the code under test.
  for _ in $(seq 1 20); do
    curl -sf -m 1 "$B/status" >/dev/null 2>&1 || break
    sleep 0.5
  done
  if curl -sf -m 1 "$B/status" >/dev/null 2>&1; then
    squatter=$(lsof -ti "TCP@127.0.0.1:$PORT" -sTCP:LISTEN 2>/dev/null | tr '\n' ' ')
    die "something else is already serving on port $PORT (pid ${squatter:-unknown}).
      stop it with:  kill ${squatter:-<pid>}
      or use another: FERRY_TEST_PORT=4321 $0"
  fi

  FERRY_DATA_DIR="$DATA" FERRY_BIND="127.0.0.1:$PORT" "$SERVER" >>"$SLOG" 2>&1 &
  SERVER_PID=$!
  SERVER_PIDS="${SERVER_PIDS:-} $SERVER_PID"
  for _ in $(seq 1 60); do
    curl -sf "$B/status" >/dev/null 2>&1 && break
    kill -0 $SERVER_PID 2>/dev/null || { tail -20 "$SLOG"; die "server exited during startup"; }
    sleep 0.5
  done
  # And prove the answer comes from the instance this run created.
  local mine serving
  mine=$(cat "$DATA/instance-id" 2>/dev/null)
  serving=$(curl -s -m 3 "$B/status" | field instance_id)
  if [ -z "$mine" ] || [ "$mine" != "$serving" ]; then
    tail -20 "$SLOG"
    die "port $PORT is answered by a different server (its instance $serving, ours ${mine:-none})"
  fi
}
start_server
echo "== server up on $PORT (pid $SERVER_PID)"

JAR="$WORK/cookies"
c() { curl -s -H "Content-Type: application/json" -H "X-Ferry-Request: 1" -b "$JAR" -c "$JAR" "$@"; }

python3 scripts/test-setup.py "$B" "$DATA" || die "platform setup failed"

c -X POST -d '{"email":"ops@x.com","password":"correct-horse-battery","name":"运维员"}' "$B/users" >/dev/null
c -X POST -d '{"name":"运维","slug":"ops"}' "$B/workspaces" >/dev/null
c -X POST -d '{"name":"另一个","slug":"other"}' "$B/workspaces" >/dev/null
HOST_A=$(c -X POST -d '{"name":"agent-01","connection":"runner"}' "$B/workspaces/ops/hosts" | field id)
HOST_B=$(c -X POST -d '{"name":"remote-01","connection":"runner"}' "$B/workspaces/other/hosts" | field id)
[ -n "$HOST_A" ] && [ -n "$HOST_B" ] || die "could not create hosts — is main.rs wired? (see the header of this script)"
echo "== workspace ops host $HOST_A / workspace other host $HOST_B"

mint() { # mint <workspace> <host-id>
  c -X POST -d "{\"host_id\":\"$2\"}" "$B/workspaces/$1/runner-tokens" | field token
}

# --------------------------------------------------------- 1. bad tokens refused

echo "== enrolment"
TOKEN=$(mint ops "$HOST_A")
[ -n "$TOKEN" ] || die "no enrolment token — is POST /{workspace}/runner-tokens wired?"
ck "a token is minted" '^[0-9a-f]\{64\}$' "$TOKEN"

run_once() { # run_once <state-dir> <token> ; prints the runner's own output
  "$RUNNER" --server "http://127.0.0.1:$PORT" --token "$2" --state-dir "$1" 2>&1 &
  local pid=$!
  ( sleep 8; kill $pid 2>/dev/null ) &
  local guard=$!
  wait $pid 2>/dev/null
  kill $guard 2>/dev/null
}
ck "garbage token refused" "注册令牌无效" "$(run_once "$WORK/s-garbage" "not-a-real-token")"

EXPIRED=$(mint ops "$HOST_A")
python3 -I -c "
import sqlite3,sys
db=sqlite3.connect(sys.argv[1]); db.execute('PRAGMA journal_mode=WAL')
db.execute('UPDATE runner_tokens SET expires_at=1 WHERE token_hash=('
           'SELECT token_hash FROM runner_tokens WHERE used_at=0 ORDER BY created_at DESC LIMIT 1)')
db.commit()" "$DATA/ferry.sqlite" || die "could not expire a token"
ck "expired token refused" "注册令牌无效" "$(run_once "$WORK/s-expired" "$EXPIRED")"

VERSION_REPLY=$(python3 -I scripts/ws-probe.py frame "ws://127.0.0.1:$PORT/api/runners/connect" \
  '{"version":99,"auth":{"type":"enroll","token":"x"},"hostname":"h","os":"linux","arch":"x86_64","agent_version":"0.0.0"}')
ck "version mismatch refused loudly" "协议版本不匹配" "$VERSION_REPLY"

# ------------------------------------------------------- 2. the real runner enrols

STATE="$WORK/state-a"
RLOG="$WORK/runner.log"
"$RUNNER" --server "http://127.0.0.1:$PORT" --token "$TOKEN" --state-dir "$STATE" >"$RLOG" 2>&1 &
RUNNER_PID=$!
for _ in $(seq 1 40); do
  [ "$(c "$B/workspaces/ops/hosts/$HOST_A" | field state)" = online ] && break
  sleep 0.5
done
ck "host goes online" "online" "$(c "$B/workspaces/ops/hosts/$HOST_A" | field state)"
ck "the key was stored" '"host_id"' "$(cat "$STATE/runner.json" 2>/dev/null)"
ck "and only for its owner" "^600$" \
  "$(python3 -I -c "import os,sys;print(oct(os.stat(sys.argv[1]).st_mode)[-3:])" "$STATE/runner.json")"
ck "re-using the same token fails" "已被使用\|注册令牌无效" "$(run_once "$WORK/s-replay" "$TOKEN")"

# ------------------------------------------------------- 3. exec, pty, listing

echo "== channel"
ck "test-connection succeeds (exec round trip, code 0)" '"ok":true' \
  "$(c -X POST "$B/workspaces/ops/hosts/$HOST_A/actions/test-connection")"

# Exercise HostChannel::exec through the existing pipeline API. There is no
# hosts/actions/exec route; skipping that nonexistent route hid these checks.
EXEC_PIPELINE=$(python3 -I -c 'import json,sys
print(json.dumps({"name":"runner-exec", "target_kind":"host", "target_ref":sys.argv[1],
 "definition":json.dumps({"workdir":sys.argv[2], "steps":[{"name":"exec", "run":"echo out; echo err 1>&2; exit 7"}]})}))' "$HOST_A" "$WORK/exec" |
  c -X POST --data-binary @- "$B/workspaces/ops/pipelines" | field id)
[ -n "$EXEC_PIPELINE" ] || die "could not create exec pipeline"
EXEC_RUN=$(c -X POST "$B/workspaces/ops/pipelines/$EXEC_PIPELINE/actions/run" | field id)
[ -n "$EXEC_RUN" ] || die "could not trigger exec pipeline"
for _ in $(seq 1 60); do
  EXEC=$(c "$B/workspaces/ops/pipeline-runs/$EXEC_RUN")
  [ "$(printf '%s' "$EXEC" | field state)" = failed ] && break
  sleep 0.5
done
EXEC_LOGS=$(c "$B/workspaces/ops/pipeline-runs/$EXEC_RUN/logs?after=0")
EXEC_STREAMS=$(printf '%s' "$EXEC_LOGS" | python3 -I -c 'import json,sys
items=json.load(sys.stdin)["items"]
for stream in ("stdout", "stderr"):
 print(stream + ":" + "".join(i["chunk"] for i in items if i["stream"] == stream).strip())')
ck "exec returns stdout" '^stdout:out$' "$EXEC_STREAMS"
ck "exec returns stderr" '^stderr:err$' "$EXEC_STREAMS"
ck "exec returns the code" '"exit_code":7' "$EXEC"

# The runner's `Session::Failed` path, which is how every remote error reaches
# the operator. Reachable without adding a route.
ck "a missing directory reports why" "无法打开目录" \
  "$(c "$B/workspaces/ops/hosts/$HOST_A/files?path=/nonexistent-ferry-path")"

XFER="$WORK/xfer"
mkdir -p "$XFER/subdir"
echo "alpha" >"$XFER/alpha.txt"
LIST=$(c "$B/workspaces/ops/hosts/$HOST_A/files?path=$XFER")
ck "list_dir finds the file"    '"name":"alpha.txt"'  "$LIST"
ck "list_dir types a directory" '"kind":"directory"'  "$LIST"
ck "list_dir reports a size"    '"size":6'            "$LIST"

COOKIE="ferry_session=$(grep ferry_session "$JAR" | awk '{print $7}' | tail -1)"
PTY=$(python3 -I scripts/ws-probe.py terminal \
  "ws://127.0.0.1:$PORT/api/workspaces/ops/hosts/$HOST_A/terminal?cols=80&rows=24" "$COOKIE")
ck "pty echoes the command's output" "hello"  "$PTY"
ck "resize reaches the tty"          "37 111" "$PTY"

# --------------------------------------- 4. a big file, both ways, flat memory

echo "== transfer of ${SIZE_MB}MB"
BIG="$XFER/big.bin"
dd if=/dev/urandom of="$WORK/source.bin" bs=1048576 count="$SIZE_MB" 2>/dev/null \
  || die "could not make the test file"
SUM_IN=$(shasum -a 256 "$WORK/source.bin" | awk '{print $1}')
rss() { ps -o rss= -p "$SERVER_PID" 2>/dev/null | tr -d ' '; }
RSS_BEFORE=$(rss)
( while kill -0 $SERVER_PID 2>/dev/null; do rss >>"$WORK/rss"; sleep 0.25; done ) &
SAMPLER=$!

UP_CODE=$(c -X PUT --data-binary "@$WORK/source.bin" -H "Content-Type: application/octet-stream" \
  "$B/workspaces/ops/hosts/$HOST_A/files/content?path=$BIG" -o /dev/null -w '%{http_code}')
ck "upload accepted" "^204$" "$UP_CODE"
ck "the file on the host matches" "^$SUM_IN$" \
  "$(shasum -a 256 "$BIG" 2>/dev/null | awk '{print $1}')"
chmod 750 "$BIG"
ck "overwrite accepted" "^204$" "$(c -X PUT --data-binary "@$WORK/source.bin" -H "Content-Type: application/octet-stream" "$B/workspaces/ops/hosts/$HOST_A/files/content?path=$BIG" -o /dev/null -w '%{http_code}')"
ck "overwrite preserves file mode" "^750$" "$(python3 -I -c 'import os,sys;print(oct(os.stat(sys.argv[1]).st_mode & 0o777)[2:])' "$BIG")"

DOWN_CODE=$(c "$B/workspaces/ops/hosts/$HOST_A/files/content?path=$BIG" \
  -o "$WORK/back.bin" -w '%{http_code}')
ck "download served" "^200$" "$DOWN_CODE"
ck "the download matches too" "^$SUM_IN$" "$(shasum -a 256 "$WORK/back.bin" | awk '{print $1}')"

kill $SAMPLER 2>/dev/null; SAMPLER=
RSS_PEAK=$(sort -n "$WORK/rss" 2>/dev/null | tail -1)
GROWTH=$(( (RSS_PEAK - RSS_BEFORE) / 1024 ))
printf '  rss  before %sKB peak %sKB (grew %sMB during a %sMB round trip)\n' \
  "$RSS_BEFORE" "$RSS_PEAK" "$GROWTH" "$SIZE_MB"
if [ "$GROWTH" -lt 64 ]; then
  ok "server memory stayed flat"
else
  bad "server grew ${GROWTH}MB on a ${SIZE_MB}MB transfer; it is buffering, not streaming"
fi

ck "deleting a non-empty directory is refused, not recursive" "请确认目录为空" \
  "$(c -X DELETE "$B/workspaces/ops/hosts/$HOST_A/files?path=$XFER")"
ck "delete removes it" "^204$" \
  "$(c -o /dev/null -w '%{http_code}' -X DELETE "$B/workspaces/ops/hosts/$HOST_A/files?path=$BIG")"
if [ -e "$BIG" ]; then bad "the file is still on the host after DELETE"; else ok "and it is really gone"; fi

# -------------------------------------------------------- 5. workspace isolation

echo "== isolation"
ck "B's own runner host is unreachable" "代理未连接" \
  "$(c "$B/workspaces/other/hosts/$HOST_B/files?path=/tmp")"
ck "A's host is invisible from B" "不存在" \
  "$(c "$B/workspaces/other/hosts/$HOST_A/files?path=/tmp")"

KEY=$(python3 -I -c "import json,sys;print(json.load(open(sys.argv[1]))['key'])" "$STATE/runner.json")
FORGED=$(python3 -I scripts/ws-probe.py frame "ws://127.0.0.1:$PORT/api/runners/connect" \
  "{\"version\":1,\"auth\":{\"type\":\"key\",\"host_id\":\"$HOST_B\",\"key\":\"$KEY\"},\"hostname\":\"h\",\"os\":\"linux\",\"arch\":\"x86_64\",\"agent_version\":\"0.1.0\"}")
ck  "A's key cannot claim B's host" "代理凭据无效" "$FORGED"
ckn "and no welcome was issued"     "welcome"      "$FORGED"

# Only an unknown session id. Claiming a session that genuinely belongs to a
# different runner needs the server-issued id, which this harness never sees,
# so that property is pinned by the unit test instead:
#   runner::tests::a_session_can_only_be_claimed_by_the_runner_it_was_asked_of
# Verified by mutation: removing the `entry.host_id == host_id` guard in
# claim_pending fails that unit test and leaves this check passing, which is
# why this one is labelled for what it actually proves.
STOLEN=$(python3 -I scripts/ws-probe.py frame "ws://127.0.0.1:$PORT/api/runners/sessions/deadbeef" \
  "{\"version\":1,\"host_id\":\"$HOST_A\",\"key\":\"$KEY\"}")
ck "an unknown session id is refused" "该会话不存在" "$STOLEN"

# ------------------------------------------------- 6. reconnect on its own

echo "== reconnect"
SEEN_BEFORE=$(c "$B/workspaces/ops/hosts/$HOST_A" | field seen_at)
kill "$SERVER_PID"; wait "$SERVER_PID" 2>/dev/null
sleep 2
if kill -0 "$RUNNER_PID" 2>/dev/null; then
  ok "runner survived the server going away"
else
  bad "runner exited when the server restarted"
fi
start_server
RECONNECTED=no
for _ in $(seq 1 60); do
  NOW=$(c "$B/workspaces/ops/hosts/$HOST_A" | field seen_at)
  if [ -n "$NOW" ] && [ "$NOW" -gt "${SEEN_BEFORE:-0}" ]; then RECONNECTED=yes; break; fi
  sleep 1
done
ck "runner reconnected by itself (seen_at advanced)" "^yes$" "$RECONNECTED"
ck "and the channel works again" '"ok":true' \
  "$(c -X POST "$B/workspaces/ops/hosts/$HOST_A/actions/test-connection")"

# ------------------------------------------------------------------- result

PANICS=$(grep -c panicked "$SLOG" 2>/dev/null)
if [ "${PANICS:-0}" -gt 0 ]; then
  grep -m3 -A3 panicked "$SLOG" | sed 's/^/    /'
  bad "$PANICS panic(s) in the server log"
fi
if [ "$pass" -lt 35 ]; then bad "expected at least 35 passed checks, got $pass"; fi
printf '\n%d passed, %d failed, 0 skipped\n' "$pass" "$fail"
if [ "$fail" -ne 0 ]; then
  echo "--- server log tail"; tail -25 "$SLOG"
  echo "--- runner log tail"; tail -25 "$RLOG"
  exit 1
fi
echo "runner channel verified end to end."
