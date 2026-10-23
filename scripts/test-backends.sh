#!/usr/bin/env bash
# Runs the same functional suite against SQLite, MySQL and PostgreSQL.
#
# The dialect layer (crates/server/src/dialect.rs) is unit-tested, but unit
# tests cannot catch the things that actually broke: Postgres numbering its
# placeholders differently, and Postgres INTEGER being i32 where SQLite hands
# back i64. Only a real server against a real database finds those, so this
# script is the gate that keeps all three working.
#
#   scripts/test-backends.sh                 # SQLite only
#   FERRY_TEST_MYSQL=mysql://...  \
#   FERRY_TEST_POSTGRES=postgres://... \
#     scripts/test-backends.sh               # all three
#
# Set FERRY_REQUIRE_ALL=1 to turn a missing URL into a failure instead of a
# skip, so CI cannot pass by quietly testing one backend.
set -uo pipefail

cd "$(dirname "$0")/.."
BINARY=${FERRY_BINARY:-target/debug/ferry-server}
[ -x "$BINARY" ] || { echo "no server binary at $BINARY; run: cargo build -p ferry-server"; exit 1; }

total_fail=0

run_suite() { # run_suite <name> <port> <database-url-or-empty>
  local name=$1 port=$2 url=$3
  local dir="${TMPDIR:-/tmp}/ferry-test-$name.$$" jar="${TMPDIR:-/tmp}/ferry-jar-$name.$$"
  local log="${TMPDIR:-/tmp}/ferry-test-$name.$$.log"
  rm -rf "$dir" "$jar"

  if [ -n "$url" ]; then
    FERRY_DATA_DIR="$dir" FERRY_BIND="127.0.0.1:$port" FERRY_DATABASE_URL="$url" "$BINARY" >"$log" 2>&1 &
  else
    FERRY_DATA_DIR="$dir" FERRY_BIND="127.0.0.1:$port" "$BINARY" >"$log" 2>&1 &
  fi
  local pid=$!
  local ready=0 i
  for i in $(seq 1 60); do
    curl -sf "http://127.0.0.1:$port/api/status" >/dev/null 2>&1 && { ready=1; break; }
    kill -0 $pid 2>/dev/null || break
    sleep 1
  done
  if [ $ready -eq 0 ]; then
    echo "  $name: server never became ready"
    sed -n '1,20p' "$log"
    kill $pid 2>/dev/null; wait $pid 2>/dev/null
    total_fail=$((total_fail + 1))
    return 1
  fi

  local B="http://127.0.0.1:$port/api" pass=0 fail=0
  c() { curl -s -H "Content-Type: application/json" -H "X-Ferry-Request: 1" -b "$jar" -c "$jar" "$@"; }
  ck() { # ck <label> <expected-regex> <actual>
    if echo "$3" | grep -q "$2"; then
      pass=$((pass + 1))
    else
      fail=$((fail + 1))
      echo "    FAIL $1: want /$2/ got: $(echo "$3" | head -c 200)"
    fi
  }

  c -X POST -d '{"email":"m@x.com","password":"correct-horse-battery","name":"测试员"}' "$B/users" >/dev/null
  ck "duplicate email"  "已注册"        "$(c -X POST -d '{"email":"m@x.com","password":"correct-horse-battery","name":"x"}' "$B/users")"
  ck "weak password"    "至少 10"       "$(c -X POST -d '{"email":"w@x.com","password":"short","name":"W"}' "$B/users")"
  ck "create workspace" '"slug":"ops"'  "$(c -X POST -d '{"name":"运维","slug":"ops"}' "$B/workspaces")"
  ck "duplicate slug"   "已被占用"      "$(c -X POST -d '{"name":"X","slug":"ops"}' "$B/workspaces")"
  ck "identity"         '"拥有者"'      "$(c "$B/me")"

  local group
  group=$(c -X POST -d '{"name":"生产组"}' "$B/workspaces/ops/host-groups" |
    python3 -I -c "import json,sys;print(json.load(sys.stdin).get('id',''))")
  [ -n "$group" ] || { echo "    FAIL could not create a host group"; fail=$((fail + 1)); }
  for i in $(seq 1 12); do
    c -X POST -d "{\"name\":\"web-$(printf %02d "$i")\",\"connection\":\"ssh\",\"address\":\"10.0.0.$i\",\"port\":22,\"username\":\"root\",\"group_id\":\"$group\"}" \
      "$B/workspaces/ops/hosts" >/dev/null
  done
  c -X POST -d '{"name":"agent-01","connection":"runner"}' "$B/workspaces/ops/hosts" >/dev/null

  ck "total"            '"total":13'    "$(c "$B/workspaces/ops/hosts?page_size=5")"
  ck "page size"        '"page_size":5' "$(c "$B/workspaces/ops/hosts?page_size=5")"
  ck "paging + sort"    'web-11'        "$(c "$B/workspaces/ops/hosts?page=3&page_size=5&sort=name")"
  ck "descending sort"  'web-12'        "$(c "$B/workspaces/ops/hosts?page_size=1&sort=-name")"
  # web-01..web-12, so the prefix web-1 matches 10, 11 and 12.
  ck "search"           '"total":3'     "$(c "$B/workspaces/ops/hosts?q=web-1")"
  ck "search is case-insensitive" '"total":3' "$(c "$B/workspaces/ops/hosts?q=WEB-1")"
  ck "filter"           '"total":12'    "$(c "$B/workspaces/ops/hosts?connection=ssh")"
  ck "two filters"      '"total":12'    "$(c "$B/workspaces/ops/hosts?connection=ssh&group_id=$group")"
  ck "comma means IN"   '"total":13'    "$(c "$B/workspaces/ops/hosts?connection=ssh,runner")"
  ck "unknown filter"   "不支持按"       "$(c "$B/workspaces/ops/hosts?evil=1")"
  ck "unknown sort"     "不支持按"       "$(c "$B/workspaces/ops/hosts?sort=password_hash")"
  ck "page past end"    '"items":\[\]'  "$(c "$B/workspaces/ops/hosts?page=99")"
  ck "ssh validation"   "需要填写主机地址" "$(c -X POST -d '{"name":"b","connection":"ssh"}' "$B/workspaces/ops/hosts")"
  ck "group in use"     "请先移出"       "$(c -X DELETE "$B/workspaces/ops/host-groups/$group")"

  local host
  host=$(c "$B/workspaces/ops/hosts?q=agent-01" |
    python3 -I -c "import json,sys;items=json.load(sys.stdin)['items'];print(items[0]['id'] if items else '')")
  ck "update"           '"ok":true'     "$(c -X PATCH -d '{"name":"agent-renamed","version":1}' "$B/workspaces/ops/hosts/$host")"
  # The second update carries the same version, so it must be refused.
  ck "stale update"     "已被他人修改"   "$(c -X PATCH -d '{"name":"again","version":1}' "$B/workspaces/ops/hosts/$host")"
  ck "read back"        'agent-renamed' "$(c "$B/workspaces/ops/hosts/$host")"

  local ids
  ids=$(c "$B/workspaces/ops/hosts?q=web-1&page_size=50" |
    python3 -I -c "import json,sys;print(json.dumps([h['id'] for h in json.load(sys.stdin)['items']]))")
  ck "bulk delete"      '"deleted":3'   "$(c -X POST -d "{\"ids\":$ids}" "$B/workspaces/ops/hosts/actions/bulk-delete")"
  ck "dashboard counts" '"hosts":10'    "$(c "$B/workspaces/ops/dashboard")"
  ck "state grouping"   '"unknown":10'  "$(c "$B/workspaces/ops/dashboard")"
  ck "activity"         '"resource":"hosts"' "$(c "$B/workspaces/ops/activity?page_size=3")"
  ck "activity sort"    '"page":1'      "$(c "$B/workspaces/ops/activity?sort=-created_at")"
  ck "create tag"       '"id"'          "$(c -X POST -d '{"name":"prod","color":"#4f9cf9"}' "$B/workspaces/ops/tags")"
  ck "roles"            '"拥有者"'      "$(c "$B/workspaces/ops/roles")"
  ck "foreign workspace is 404" "不存在" "$(c "$B/workspaces/nope/hosts")"

  kill $pid 2>/dev/null; wait $pid 2>/dev/null

  # A panic is a failure even when the HTTP response looked fine: the i32/i64
  # mismatch on Postgres surfaced exactly this way.
  local panics
  # grep -c already prints 0 when it matches nothing; `|| echo 0` would append
  # a second line and make the comparison below a syntax error.
  panics=$(grep -c panicked "$log" 2>/dev/null)
  if [ "$panics" -gt 0 ]; then
    echo "    FAIL $panics panic(s) in the server log:"
    grep -m3 -A2 panicked "$log" | sed 's/^/      /'
    fail=$((fail + 1))
  fi

  printf '  %-10s %2d passed, %d failed\n' "$name:" "$pass" "$fail"
  rm -rf "$dir" "$jar" "$log"
  total_fail=$((total_fail + fail))
}

require() { # require <name> <url>
  if [ -n "$2" ]; then
    return 0
  elif [ "${FERRY_REQUIRE_ALL:-0}" = "1" ]; then
    echo "  $1: no URL configured and FERRY_REQUIRE_ALL=1"
    total_fail=$((total_fail + 1))
    return 1
  else
    echo "  $1: skipped (no URL configured)"
    return 1
  fi
}

run_suite SQLite 4310 ""
require MySQL "${FERRY_TEST_MYSQL:-}" && run_suite MySQL 4311 "$FERRY_TEST_MYSQL"
require Postgres "${FERRY_TEST_POSTGRES:-}" && run_suite Postgres 4312 "$FERRY_TEST_POSTGRES"

if [ "$total_fail" -ne 0 ]; then
  echo "FAILED: $total_fail check(s)"
  exit 1
fi
echo "All backends passed."
