#!/usr/bin/env bash
# Runs the same functional suite against SQLite, MySQL and PostgreSQL.
#
# The migration builders are unit-tested, but unit tests cannot catch all
# backend differences: Postgres INTEGER is i32 where SQLite hands back i64.
# Only a real server against a real database finds those, so this script is
# the gate that keeps all three working.
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
# Build here rather than trusting whatever binary happens to be lying around.
# A failed build used to leave the previous binary in place, and the whole
# suite then passed against stale code — which is indistinguishable from the
# change actually working. Set FERRY_BINARY to skip (e.g. a release build).
if [ -z "${FERRY_BINARY:-}" ]; then
  echo "building $BINARY"
  if ! cargo build -p ferry-server; then
    echo "FAILED: the server did not build; refusing to test a stale binary."
    exit 1
  fi
fi
[ -x "$BINARY" ] || { echo "no server binary at $BINARY"; exit 1; }

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
  ckn() { # ckn <label> <regex-that-must-NOT-appear> <actual>
    if echo "$3" | grep -q "$2"; then
      fail=$((fail + 1))
      echo "    FAIL $1: /$2/ must not appear, got: $(echo "$3" | head -c 200)"
    else
      pass=$((pass + 1))
    fi
  }
  ck() { # ck <label> <expected-regex> <actual>
    if echo "$3" | grep -q "$2"; then
      pass=$((pass + 1))
    else
      fail=$((fail + 1))
      echo "    FAIL $1: want /$2/ got: $(echo "$3" | head -c 200)"
    fi
  }

  python3 scripts/test-setup.py "$B" "$dir" || { kill "$pid"; wait "$pid"; return 1; }

  c -X POST -d '{"email":"m@x.com","password":"correct-horse-battery","name":"测试员"}' "$B/users" >/dev/null
  ck "duplicate email"  "已注册"        "$(c -X POST -d '{"email":"m@x.com","password":"correct-horse-battery","name":"x"}' "$B/users")"
  ck "weak password"    "至少 10"       "$(c -X POST -d '{"email":"w@x.com","password":"short","name":"W"}' "$B/users")"
  ck "create workspace" '"slug":"ops"'  "$(c -X POST -d '{"name":"运维","slug":"ops"}' "$B/workspaces")"
  ck "duplicate slug"   "已被占用"      "$(c -X POST -d '{"name":"X","slug":"ops"}' "$B/workspaces")"
  ck "identity"         '"拥有者"'      "$(c "$B/me")"

  # The session cookie must be a random secret, not the row's primary key. A
  # snowflake encodes its creation time and a small counter, so a cookie that
  # was one would be guessable from roughly when its owner signed in.
  local cookie_value
  cookie_value=$(grep -oE 'ferry_session[[:space:]]+[^[:space:]]+$' "$jar" | awk '{print $NF}')
  if [ "${#cookie_value}" -ge 32 ] && ! echo "$cookie_value" | grep -qE '^[0-9]+$'; then
    pass=$((pass + 1))
  else
    fail=$((fail + 1))
    echo "    FAIL session cookie looks guessable: '${cookie_value}' (len ${#cookie_value})"
  fi

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

  # Credentials: the secret goes in and must never come back out. A long
  # multi-line value is used on purpose — MySQL's default VARCHAR(255) would
  # truncate a real private key, and the failure would only appear later as
  # "cannot decrypt".
  local pem cred
  pem="-----BEGIN OPENSSH PRIVATE KEY-----\n$(printf 'b3BlbnNzaC1rZXktdjEAAAAA%.0s' $(seq 1 40))\n-----END OPENSSH PRIVATE KEY-----"
  cred=$(c -X POST -d "{\"name\":\"prod-key\",\"kind\":\"ssh-key\",\"username\":\"root\",\"secret\":\"$pem\"}" "$B/workspaces/ops/credentials" |
    python3 -I -c "import json,sys;print(json.load(sys.stdin).get('id',''))")
  ck "create credential"  '"has_secret":true'  "$(c "$B/workspaces/ops/credentials/$cred")"
  ckn "secret never returned by detail"  'BEGIN OPENSSH'  "$(c "$B/workspaces/ops/credentials/$cred")"
  ckn "secret never returned by list"    'BEGIN OPENSSH'  "$(c "$B/workspaces/ops/credentials")"
  ck "credential needs a secret"  "请填写凭据内容"  "$(c -X POST -d '{"name":"empty","kind":"password"}' "$B/workspaces/ops/credentials")"
  ck "credential needs a known kind"  "请选择凭据类型"  "$(c -X POST -d '{"name":"x","kind":"magic","secret":"y"}' "$B/workspaces/ops/credentials")"
  # Editing without a secret must keep the stored one rather than blanking it.
  ck "rename keeps the secret"  '"ok":true'  "$(c -X PATCH -d '{"name":"prod-key-2","version":1}' "$B/workspaces/ops/credentials/$cred")"
  ck "and it is still set"      '"has_secret":true'  "$(c "$B/workspaces/ops/credentials/$cred")"
  # A host in use pins its credential.
  c -X POST -d "{\"name\":\"ssh-host\",\"connection\":\"ssh\",\"address\":\"10.0.0.99\",\"port\":22,\"username\":\"root\",\"credential_id\":\"$cred\"}" "$B/workspaces/ops/hosts" >/dev/null
  ck "credential in use cannot be deleted"  "还有 1 台主机"  "$(c -X DELETE "$B/workspaces/ops/credentials/$cred")"
  # A repository pins its credential too; the host check alone used to let a
  # repository keep a reference to a deleted credential.
  local repo_cred
  repo_cred=$(c -X POST -d '{"name":"git-token","kind":"token","secret":"ghp_example"}' "$B/workspaces/ops/credentials" |
    python3 -I -c "import json,sys;print(json.load(sys.stdin).get('id',''))")
  c -X POST -d "{\"name\":\"pinned\",\"url\":\"https://example.com/a.git\",\"credential_id\":\"$repo_cred\"}" "$B/workspaces/ops/repositories" >/dev/null
  # A pipeline pins its target host; both the single and the batch delete must
  # refuse, not just the one named in the report. The payload is built with
  # python because the definition is JSON nested inside JSON and bash quoting
  # silently produced an empty id the first time.
  local pinned_host pipeline_payload
  pinned_host=$(c -X POST -d '{"name":"pipeline-target","connection":"runner"}' "$B/workspaces/ops/hosts" |
    python3 -I -c "import json,sys;print(json.load(sys.stdin).get('id',''))")
  [ -n "$pinned_host" ] || { echo "    FAIL could not create the pinned host"; fail=$((fail + 1)); }
  pipeline_payload=$(python3 -I -c "
import json,sys
print(json.dumps({
  'name': 'pins-host',
  'definition': json.dumps({'workdir': '/tmp/ferry', 'steps': [{'name': 's', 'run': 'true'}]}),
  'target_kind': 'host',
  'target_ref': sys.argv[1],
}))" "$pinned_host")
  ck "pipeline pinning a host is created" '"id"' "$(c -X POST -d "$pipeline_payload" "$B/workspaces/ops/pipelines")"
  ck "host used by a pipeline cannot be deleted"  "条流水线以这台主机为目标"  "$(c -X DELETE "$B/workspaces/ops/hosts/$pinned_host")"
  ck "nor by the batch delete"                    "条流水线以这台主机为目标"  "$(c -X POST -d "{\"ids\":[\"$pinned_host\"]}" "$B/workspaces/ops/hosts/actions/bulk-delete")"

  ck "credential used by a repository cannot be deleted"  "还有 1 个仓库"  "$(c -X DELETE "$B/workspaces/ops/credentials/$repo_cred")"
  # Repositories share credentials but never include credential data in audit details.
  local repo repo_body
  repo_body=$(c -X POST -d '{"name":"repo-public","url":"https://example.com/a/b.git"}' "$B/workspaces/ops/repositories")
  ck "create repository" '"id"' "$repo_body"
  repo=$(printf '%s' "$repo_body" | python3 -I -c "import json,sys;print(json.load(sys.stdin).get('id',''))")
  ck "repository default branch" '"default_branch":"main"' "$(c "$B/workspaces/ops/repositories/$repo")"
  ck "repository local URL rejected" '"error"' "$(c -X POST -d '{"name":"local","url":"file:///etc"}' "$B/workspaces/ops/repositories")"
  ck "repository missing credential rejected" '"error"' "$(c -X POST -d '{"name":"missing","url":"https://example.com/repo","credential_id":"missing"}' "$B/workspaces/ops/repositories")"
  ck "repository real credential accepted" '"id"' "$(c -X POST -d "{\"name\":\"repo-private\",\"url\":\"git@github.com:a/b.git\",\"credential_id\":\"$cred\"}" "$B/workspaces/ops/repositories")"
  ck "repository name search" '"total":1' "$(c "$B/workspaces/ops/repositories?q=repo-public")"
  ck "repository URL search" '"total":1' "$(c "$B/workspaces/ops/repositories?q=github.com")"
  ck "repository update" '"ok":true' "$(c -X PATCH -d '{"name":"repo-updated","url":"ssh://git@example.com/repo","version":1}' "$B/workspaces/ops/repositories/$repo")"
  ck "repository stale update" '409$' "$(c -w '%{http_code}' -X PATCH -d '{"name":"again","url":"https://example.com/repo","version":1}' "$B/workspaces/ops/repositories/$repo")"
  ck "repository update validates credential" '"error"' "$(c -X PATCH -d '{"name":"again","url":"https://example.com/repo","credential_id":"missing","version":2}' "$B/workspaces/ops/repositories/$repo")"
  ck "repository delete" '204$' "$(c -w '%{http_code}' -X DELETE "$B/workspaces/ops/repositories/$repo")"
  ck "repository deleted detail" '404$' "$(c -w '%{http_code}' "$B/workspaces/ops/repositories/$repo")"
  ck "repository audit exists" '"resource":"repositories"' "$(c "$B/workspaces/ops/activity?resource=repositories")"
  ck "repository second workspace" '"slug":"repo-other"' "$(c -X POST -d '{"name":"Other","slug":"repo-other"}' "$B/workspaces")"
  ck "repository foreign credential rejected" '"error"' "$(c -X POST -d "{\"name\":\"foreign\",\"url\":\"https://example.com/repo\",\"credential_id\":\"$cred\"}" "$B/workspaces/repo-other/repositories")"
  ck "repository workspace isolation" '"total":0' "$(c "$B/workspaces/repo-other/repositories")"
  ckn "repository audit excludes credential references" 'credential_id' "$(c "$B/workspaces/ops/activity?resource=repositories")"


  # Shared variables: exercise the actual HTTP boundary on every database.
  local variable variable_body revealed
  variable_body=$(c -X POST -d '{"name":"db-url","scope":"global","kind":"secret","value":"  variable-test-secret\n$literal  "}' "$B/workspaces/ops/variables")
  ck "variable create" '"id"' "$variable_body"
  variable=$(printf '%s' "$variable_body" | python3 -I -c 'import json,sys; print(json.load(sys.stdin).get("id",""))')
  ck "variable presence" '"has_value":true' "$(c "$B/workspaces/ops/variables/$variable")"
  ckn "variable detail does not expose plaintext or value field" 'variable-test-secret\|"value":' "$(c "$B/workspaces/ops/variables/$variable")"
  ckn "variable list does not expose plaintext or value field" 'variable-test-secret\|"value":' "$(c "$B/workspaces/ops/variables?scope=global&scope_ref=0")"
  ck "variable duplicate" '409$' "$(c -w '%{http_code}' -X POST -d '{"name":"db-url","value":"other"}' "$B/workspaces/ops/variables")"
  ck "variable other workspace cannot read" '404$' "$(c -w '%{http_code}' "$B/workspaces/repo-other/variables/$variable")"
  ck "variable other workspace cannot reveal" '404$' "$(c -w '%{http_code}' -X POST -d '{}' "$B/workspaces/repo-other/variables/$variable/actions/reveal")"
  ck "variable missing scope object" '400$' "$(c -w '%{http_code}' -X POST -d '{"name":"object","scope":"deploy","scope_ref":"999","value":"x"}' "$B/workspaces/ops/variables")"
  revealed=$(c -X POST -d '{}' "$B/workspaces/ops/variables/$variable/actions/reveal")
  ck "variable exact whitespace multiline and dollar" '^True$' "$(printf '%s' "$revealed" | python3 -I -c 'import json,sys; print(json.load(sys.stdin)["value"] == "  variable-test-secret\n$literal  ")')"
  ck "variable reveal audited" '"action":"reveal"' "$(c "$B/workspaces/ops/activity?resource=variables")"
  ckn "variable audit excludes content" 'variable-test-secret' "$(c "$B/workspaces/ops/activity?resource=variables")"
  ck "variable reveal is not cacheable" 'no-store' "$(c -D - -X POST -d '{}' "$B/workspaces/ops/variables/$variable/actions/reveal")"
  ck "variable update keeps value when omitted" '"ok":true' "$(c -X PATCH -d '{"name":"db-url","version":1,"description":"updated"}' "$B/workspaces/ops/variables/$variable")"
  ck "variable value preserved" 'variable-test-secret' "$(c -X POST -d '{}' "$B/workspaces/ops/variables/$variable/actions/reveal")"
  ck "variable explicit empty replacement" '"ok":true' "$(c -X PATCH -d '{"name":"db-url","version":2,"value":""}' "$B/workspaces/ops/variables/$variable")"
  ck "variable empty reveal" '"value":""' "$(c -X POST -d '{}' "$B/workspaces/ops/variables/$variable/actions/reveal")"
  ck "variable stale edit" '409$' "$(c -w '%{http_code}' -X PATCH -d '{"name":"db-url","version":2,"value":"stale"}' "$B/workspaces/ops/variables/$variable")"
  ck "variable immutable type" '400$' "$(c -w '%{http_code}' -X PATCH -d '{"name":"db-url","version":3,"kind":"plain"}' "$B/workspaces/ops/variables/$variable")"
  ck "variable deletion" '204$' "$(c -w '%{http_code}' -X DELETE "$B/workspaces/ops/variables/$variable")"
  ck "variable deleted detail" '404$' "$(c -w '%{http_code}' "$B/workspaces/ops/variables/$variable")"

  # Pipelines: keep the JSON definition a string at the resource boundary.
  local pipeline pipeline_body pipeline_payload pipeline_repo foreign_repo
  pipeline_repo=$(c -X POST -d '{"name":"pipeline-repo","url":"https://example.com/pipeline.git"}' "$B/workspaces/ops/repositories" |
    python3 -I -c "import json,sys;print(json.load(sys.stdin).get('id',''))")
  foreign_repo=$(c -X POST -d '{"name":"foreign-repo","url":"https://example.com/foreign.git"}' "$B/workspaces/repo-other/repositories" |
    python3 -I -c "import json,sys;print(json.load(sys.stdin).get('id',''))")
  pipeline_payload=$(python3 -I -c '
import json,sys
definition={"workdir":"/tmp/{{run}}","env":[{"name":"LONG_VALUE","value":"x"*300}],
            "steps":[{"name":"构建","run":"echo {{branch}} {{repo_url}} {{commit}}","timeout_seconds":1800}]}
print(json.dumps({"name":"pipeline-build","repository_id":sys.argv[1],"target_kind":"host",
                  "target_ref":sys.argv[2],"definition":json.dumps(definition),"version":1}))
' "$pipeline_repo" "$host")
  pipeline_body=$(c -X POST -d "$pipeline_payload" "$B/workspaces/ops/pipelines")
  ck "pipeline create" '"id"' "$pipeline_body"
  pipeline=$(printf '%s' "$pipeline_body" | python3 -I -c "import json,sys;print(json.load(sys.stdin).get('id',''))")
  ck "pipeline long definition round trip" '"LONG_VALUE"' "$(c "$B/workspaces/ops/pipelines/$pipeline" |
    python3 -I -c 'import json,sys;v=json.load(sys.stdin);d=json.loads(v["definition"]);assert d["env"][0]["value"]=="x"*300;print(json.dumps(d))')"
  # Reuse the valid payload so each refusal exercises exactly one changed field.
  pipeline_variant() {
    printf '%s' "$pipeline_payload" | python3 -I -c 'import json,sys;v=json.load(sys.stdin);v[sys.argv[1]]=sys.argv[2];print(json.dumps(v))' "$1" "$2"
  }
  ck "pipeline malformed JSON" '400$' "$(c -w '%{http_code}' -X POST -d "$(pipeline_variant definition '{')" "$B/workspaces/ops/pipelines")"
  ck "pipeline foreign repository" '400$' "$(c -w '%{http_code}' -X POST -d "$(pipeline_variant repository_id "$foreign_repo")" "$B/workspaces/ops/pipelines")"
  ck "pipeline missing repository" '400$' "$(c -w '%{http_code}' -X POST -d "$(pipeline_variant repository_id missing)" "$B/workspaces/ops/pipelines")"
  ck "pipeline missing host" '400$' "$(c -w '%{http_code}' -X POST -d "$(pipeline_variant target_ref missing)" "$B/workspaces/ops/pipelines")"
  ck "pipeline invalid target kind" '400$' "$(c -w '%{http_code}' -X POST -d "$(pipeline_variant target_kind invalid)" "$B/workspaces/ops/pipelines")"
  ck "pipeline typo names variable" '{{typo}}.*400$' "$(c -w '%{http_code}' -X POST -d "$(pipeline_variant definition '{"workdir":"/tmp","steps":[{"name":"构建","run":"echo {{typo}}"}]}')" "$B/workspaces/ops/pipelines")"
  ck "pipeline search and filters" '"total":1' "$(c "$B/workspaces/ops/pipelines?q=PIPELINE-BUILD&repository_id=$pipeline_repo&target_kind=host&sort=-updated_at")"
  ck "pipeline filter excludes" '"total":0' "$(c "$B/workspaces/ops/pipelines?target_kind=host-group")"
  ck "pipeline unknown sort" '400$' "$(c -w '%{http_code}' "$B/workspaces/ops/pipelines?sort=definition")"
  ck "pipeline update" '200$' "$(c -w '%{http_code}' -X PATCH -d "$pipeline_payload" "$B/workspaces/ops/pipelines/$pipeline")"
  ck "pipeline stale update" '409$' "$(c -w '%{http_code}' -X PATCH -d "$pipeline_payload" "$B/workspaces/ops/pipelines/$pipeline")"
  ck "pipeline version incremented" '"version":2' "$(c "$B/workspaces/ops/pipelines/$pipeline")"
  # The successful update above advanced the version; exercise reference
  # validation with a current version, independently of stale-write rejection.
  pipeline_payload=$(printf '%s' "$pipeline_payload" | python3 -I -c 'import json,sys;v=json.load(sys.stdin);v["version"]=2;print(json.dumps(v))')
  ck "pipeline update validates references" '400$' "$(c -w '%{http_code}' -X PATCH -d "$(pipeline_variant repository_id "$foreign_repo")" "$B/workspaces/ops/pipelines/$pipeline")"
  ck "pipeline referenced repository protected" '409$' "$(c -w '%{http_code}' -X DELETE "$B/workspaces/ops/repositories/$pipeline_repo")"
  ck "pipeline isolated list" '"total":0' "$(c "$B/workspaces/repo-other/pipelines")"
  ck "pipeline isolated detail" '404$' "$(c -w '%{http_code}' "$B/workspaces/repo-other/pipelines/$pipeline")"
  ck "pipeline isolated delete" '404$' "$(c -w '%{http_code}' -X DELETE "$B/workspaces/repo-other/pipelines/$pipeline")"

  local group_payload group_pipeline other_host other_group other_payload
  other_host=$(c -X POST -d '{"name":"other-host","connection":"runner"}' "$B/workspaces/repo-other/hosts" |
    python3 -I -c "import json,sys;print(json.load(sys.stdin).get('id',''))")
  other_group=$(c -X POST -d '{"name":"other-group"}' "$B/workspaces/repo-other/host-groups" |
    python3 -I -c "import json,sys;print(json.load(sys.stdin).get('id',''))")
  ck "pipeline foreign host" '400$' "$(c -w '%{http_code}' -X POST -d "$(pipeline_variant target_ref "$other_host")" "$B/workspaces/ops/pipelines")"
  group_payload=$(printf '%s' "$pipeline_payload" | python3 -I -c '
import json,sys
v=json.load(sys.stdin);v.update(name="group-build",repository_id="",target_kind="host-group",target_ref=sys.argv[1]);print(json.dumps(v))
' "$group")
  group_pipeline=$(c -X POST -d "$group_payload" "$B/workspaces/ops/pipelines" |
    python3 -I -c "import json,sys;print(json.load(sys.stdin).get('id',''))")
  ck "pipeline group and optional repository accepted" '"target_kind":"host-group"' "$(c "$B/workspaces/ops/pipelines/$group_pipeline")"
  for target in missing "$other_group" "$host"; do
    local invalid_group
    invalid_group=$(printf '%s' "$group_payload" | python3 -I -c 'import json,sys;v=json.load(sys.stdin);v["target_ref"]=sys.argv[1];print(json.dumps(v))' "$target")
    ck "pipeline missing foreign or wrong-table group" '400$' "$(c -w '%{http_code}' -X POST -d "$invalid_group" "$B/workspaces/ops/pipelines")"
  done
  other_payload=$(printf '%s' "$group_payload" | python3 -I -c 'import json,sys;v=json.load(sys.stdin);v["target_ref"]=sys.argv[1];print(json.dumps(v))' "$other_group")
  ck "pipeline isolated update" '404$' "$(c -w '%{http_code}' -X PATCH -d "$other_payload" "$B/workspaces/repo-other/pipelines/$pipeline")"
  ck "pipeline group delete" '204$' "$(c -w '%{http_code}' -X DELETE "$B/workspaces/ops/pipelines/$group_pipeline")"
  ck "pipeline delete" '204$' "$(c -w '%{http_code}' -X DELETE "$B/workspaces/ops/pipelines/$pipeline")"
  ck "pipeline deleted detail" '404$' "$(c -w '%{http_code}' "$B/workspaces/ops/pipelines/$pipeline")"
  ck "pipeline unreferenced repository delete" '204$' "$(c -w '%{http_code}' -X DELETE "$B/workspaces/ops/repositories/$pipeline_repo")"
  ck "pipeline audit exists" '"resource":"pipelines"' "$(c "$B/workspaces/ops/activity?resource=pipelines")"
  ckn "pipeline audit omits definition" 'LONG_VALUE' "$(c "$B/workspaces/ops/activity?resource=pipelines")"

  ck "roles"            '"拥有者"'      "$(c "$B/workspaces/ops/roles")"
  ck "foreign workspace is 404" "不存在" "$(c "$B/workspaces/nope/hosts")"

  # T3b: inspect JSON types, not regexes that accept both numbers and strings.
  # The real generator is already >2^53; assert that premise rather than
  # silently letting a small-ID run stand in for the precision regression.
  ck_json() { # label, Python assertion, JSON, optional comparison arguments
    local label=$1 assertion=$2 payload=$3
    shift 3
    if python3 -I -c 'import json,sys;v=json.loads(sys.argv[1]);exec(sys.argv[2])' "$payload" "$assertion" "$@"; then
      pass=$((pass + 1))
    else
      fail=$((fail + 1))
      echo "    FAIL $label"
    fi
  }
  local bigint_body bigint_id bigint_detail
  bigint_body=$(c -X POST -d "{\"name\":\"bigint-roundtrip\",\"connection\":\"runner\",\"group_id\":\"$group\"}" "$B/workspaces/ops/hosts")
  ck_json "bigint create ID is a string above 2^53" 'assert type(v["id"]) is str and int(v["id"]) > 2**53' "$bigint_body"
  bigint_id=$(printf '%s' "$bigint_body" | python3 -I -c 'import json,sys;print(json.load(sys.stdin)["id"])')
  ck_json "bigint list preserves exact string ID and reference" 'assert v["total"] == 1; h=v["items"][0]; assert h["id"] == sys.argv[3] and h["group_id"] == sys.argv[4]' "$(c "$B/workspaces/ops/hosts?q=bigint-roundtrip")" "$bigint_id" "$group"
  bigint_detail=$(c "$B/workspaces/ops/hosts/$bigint_id")
  ck_json "bigint detail ID is a string and string group_id associates" 'assert v["id"] == sys.argv[3] and v["group_id"] == sys.argv[4] and v["credential_id"] == "0"' "$bigint_detail" "$bigint_id" "$group"
  ck "bigint update" '200$' "$(c -w '%{http_code}' -X PATCH -d "{\"name\":\"bigint-updated\",\"group_id\":\"$group\",\"version\":1}" "$B/workspaces/ops/hosts/$bigint_id")"
  ck_json "bigint updated detail keeps exact ID" 'assert v["id"] == sys.argv[3] and v["name"] == "bigint-updated" and v["version"] == 2' "$(c "$B/workspaces/ops/hosts/$bigint_id")" "$bigint_id"
  ck "bigint delete" '204$' "$(c -w '%{http_code}' -X DELETE "$B/workspaces/ops/hosts/$bigint_id")"
  ck "bigint deleted detail" '404$' "$(c -w '%{http_code}' "$B/workspaces/ops/hosts/$bigint_id")"
  ck "nonnumeric host path" '404$' "$(c -w '%{http_code}' "$B/workspaces/ops/hosts/not-a-number")"

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
