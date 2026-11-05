"""SQLite/document reference fixtures for storage design, never a runtime adapter."""
from pathlib import Path
from functools import cmp_to_key
import hashlib
import json
import re
import sqlite3

ROOT = Path(__file__).resolve().parents[3]
HERE = ROOT / 'src/shared/workflow-contracts'


def run():
    model_path = ROOT / 'docs/prd/ferry-工作流平台数据模型.md'
    model = model_path.read_text(encoding='utf-8')
    contract = json.loads((HERE / 'workflow-data-nodes.json').read_text(encoding='utf-8'))['storage_contract']
    prefix_bytes = contract['text_prefix_bytes']
    assert prefix_bytes == 128
    db = sqlite3.connect(':memory:')
    target = model.split('### 10.10 完整目标表DDL', 1)[1].split('### 10.10a', 1)[0]
    allowed = ('data_table_rows', 'vector_items')
    for table in allowed:
        statement = re.search(r'CREATE TABLE '+table+r' \(.*?\n\);', target, re.S)
        assert statement, table
        db.executescript(statement[0])
    for statement in re.findall(r'CREATE (?:UNIQUE )?INDEX .*?;', target):
        if any(' ON '+table+'(' in statement for table in allowed):
            db.executescript(statement)
    cases = []

    def check(name, function):
        function()
        cases.append({'case': name, 'result': 'PASS'})

    def equal(actual, expected):
        assert actual == expected, (actual, expected)

    def prefix(value):
        return value.encode('utf-8', errors='strict')[:prefix_bytes]

    def rejected(function):
        try:
            function()
        except (ValueError, sqlite3.IntegrityError):
            return
        raise AssertionError('expected rejection')

    def insert_row(row_id, value, workspace=1, table=1):
        db.execute('INSERT INTO data_table_rows(id,workspace_id,table_id,data,i1,created_at,updated_at) VALUES (?,?,?,?,?,0,0)',
                   (row_id, workspace, table, json.dumps({'v': value}, ensure_ascii=False), prefix(value)))

    values = {1: 'a'*128+'z', 2: 'a'*128+'b', 3: 'a'*128+'b', 4: '',
              5: '中'*43+'甲', 6: '中'*43+'乙', 7: '😀'*33+'a', 8: '😀'*33+'b',
              9: 'é', 10: 'e\u0301', 11: 'A', 12: 'a', 13: 'a\x00b', 14: 'a'*129}
    for row_id, value in values.items():
        insert_row(row_id, value)
    insert_row(101, values[2], workspace=2)
    insert_row(102, values[2], table=2)
    check('empty prefix stays nonnull binary', lambda: equal(db.execute('SELECT typeof(i1),length(i1) FROM data_table_rows WHERE id=4').fetchone(), ('blob', 0)))
    check('128 byte boundary', lambda: equal(len(prefix('😀'*33)), 128))
    check('multibyte prefix may end inside UTF8 sequence', lambda: rejected(lambda: prefix('中'*43).decode('utf-8')))
    check('unpaired surrogate rejected', lambda: rejected(lambda: prefix('\ud800')))
    check('Unicode normalization not implicit', lambda: equal(prefix(values[9]) == prefix(values[10]), False))
    check('case folding not implicit', lambda: equal(prefix(values[11]) == prefix(values[12]), False))

    sql_relation = {'eq': '=', 'gt': '>=', 'gte': '>=', 'lt': '<=', 'lte': '<='}
    def matches(value, query, relation):
        a, b = value.encode('utf-8'), query.encode('utf-8')
        return {'eq': a == b, 'ne': a != b, 'gt': a > b, 'gte': a >= b, 'lt': a < b, 'lte': a <= b}[relation]

    def candidates(query, relation):
        predicate = ' AND i1'+sql_relation[relation]+'?' if relation in sql_relation else ' AND i1 IS NOT NULL'
        parameters = [1, 1] + ([prefix(query)] if relation in sql_relation else [])
        return db.execute('SELECT id,data FROM data_table_rows WHERE workspace_id=? AND table_id=?'+predicate+' ORDER BY id', parameters).fetchall()

    for relation in ('eq', 'ne', 'gt', 'gte', 'lt', 'lte'):
        def compare(r=relation):
            query = values[2]
            actual = [rid for rid, data in candidates(query, r) if matches(json.loads(data)['v'], query, r)]
            expected = [rid for rid, value in values.items() if matches(value, query, r)]
            equal(actual, expected)
        check('full predicate after prefix: '+relation, compare)

    def page():
        query = values[2]
        rows = candidates(query, 'eq')
        # First candidate is a prefix collision, so SQL LIMIT 1 is not a final page.
        equal(rows[0][0], 1)
        consumed, result = 0, []
        for rid, data in rows:
            consumed = rid
            if json.loads(data)['v'] == query:
                result.append(rid)
                if len(result) == 1:
                    break
        equal((result, consumed), ([2], 2))
        second = [rid for rid, data in rows if rid > consumed and json.loads(data)['v'] == query][:1]
        equal(second, [3])
        equal(sum(json.loads(data)['v'] == query for rid, data in rows), 2)
    check('residual filtering before page/count/cursor', page)

    def sorted_values(descending):
        # Ordered prefixes are only candidates; complete equal-prefix buckets
        # must be ordered by the original bytes before a final LIMIT.
        buckets = {}
        for rid, value in values.items():
            buckets.setdefault(prefix(value), []).append((rid, value))
        output = []
        def compare(a, b):
            av, bv = a[1].encode('utf-8'), b[1].encode('utf-8')
            order = (av > bv) - (av < bv)
            return (-order if descending else order) or ((a[0] > b[0]) - (a[0] < b[0]))
        for key in sorted(buckets, reverse=descending):
            output.extend(sorted(buckets[key], key=cmp_to_key(compare)))
        return [rid for rid, value in output], [rid for rid, value in sorted(values.items(), key=cmp_to_key(compare))]
    for descending in (False, True):
        check('complete prefix buckets sort '+str(descending), lambda d=descending: equal(*sorted_values(d)))

    def null_missing():
        for rid, data in ((201, '{}'), (202, '{"v":null}')):
            db.execute('INSERT INTO data_table_rows(id,workspace_id,table_id,data,created_at,updated_at) VALUES (?,1,1,?,0,0)', (rid, data))
        rows = db.execute('SELECT id,data FROM data_table_rows WHERE workspace_id=1 AND table_id=1 AND i1 IS NULL').fetchall()
        equal([rid for rid, data in rows if 'v' in json.loads(data) and json.loads(data)['v'] is None], [202])
    check('missing and JSON null need original data', null_missing)
    check('text ne does not treat null/missing as text', lambda: equal(
        [rid for rid, data in candidates(values[2], 'ne') if json.loads(data)['v'] != values[2]],
        [rid for rid, value in values.items() if value != values[2]]))
    def empty_order(nulls):
        rows = db.execute('SELECT id,data FROM data_table_rows WHERE workspace_id=1 AND table_id=1').fetchall()
        empty = sorted(rid for rid, data in rows if json.loads(data).get('v') is None)
        strings = [rid for rid, value in sorted(values.items(), key=lambda pair: (pair[1].encode('utf-8'), pair[0]))]
        return empty+strings if nulls == 'first' else strings+empty
    check('explicit nulls first with full string ordering', lambda: equal(empty_order('first')[:3], [201, 202, 4]))
    check('explicit nulls last with full string ordering', lambda: equal(empty_order('last')[-2:], [201, 202]))
    check('embedded NUL preserved', lambda: equal(json.loads(db.execute('SELECT data FROM data_table_rows WHERE id=13').fetchone()[0])['v'], 'a\x00b'))
    check('oversized prefix rejected', lambda: rejected(lambda: db.execute('UPDATE data_table_rows SET i1=? WHERE id=1', (b'x'*129,))))
    check('TEXT binding cannot masquerade as binary prefix', lambda: rejected(lambda: db.execute('UPDATE data_table_rows SET i1=? WHERE id=1', ('abc',))))

    bucket_sql = model.split('### 13.3 ', 1)[1].split('```sql\n', 1)[1].split('```', 1)[0]
    def put_key(key, workspace=1, collection=1, forced_hash=None):
        # Sequential reference fixture: real adapters must take the collection
        # lock around this entire lookup/compare/allocate/write sequence.
        digest = forced_hash if forced_hash is not None else hashlib.sha256(key.encode('utf-8')).digest()
        rows = db.execute(bucket_sql, {'workspace': workspace, 'collection_id': collection, 'item_key_hash': digest}).fetchall()
        for rid, stored_key, slot, version in rows:
            if json.loads(stored_key).encode('utf-8') == key.encode('utf-8'):
                db.execute('UPDATE vector_items SET version=version+1 WHERE id=? AND version=?', (rid, version))
                return rid, False
        slot = max((row[2] for row in rows), default=-1) + 1
        rid = db.execute('SELECT COALESCE(MAX(id),0)+1 FROM vector_items').fetchone()[0]
        db.execute('INSERT INTO vector_items(id,workspace_id,collection_id,item_key,item_key_hash,key_slot,embedding,metadata,created_at,updated_at) VALUES (?,?,?,?,?,?,?,\'{}\',0,0)',
                   (rid, workspace, collection, json.dumps(key,ensure_ascii=False,separators=(',',':')), digest, slot, b'\0'*8))
        return rid, True

    collision = b'h'*32
    first = put_key('中'*2000+'a', forced_hash=collision)
    second = put_key('中'*2000+'b', forced_hash=collision)
    check('forced hash collision preserves two full keys', lambda: equal((first[1], second[1], first[0] != second[0]), (True, True, True)))
    check('same full key returns existing slot on replay', lambda: equal(put_key('中'*2000+'a', forced_hash=collision), (first[0], False)))
    check('hash collision slots distinct', lambda: equal(db.execute('SELECT key_slot FROM vector_items WHERE workspace_id=1 AND collection_id=1 ORDER BY key_slot').fetchall(), [(0,), (1,)]))
    check('hash bucket workspace isolated', lambda: equal(put_key('中'*2000+'a', workspace=2, forced_hash=collision)[1], True))
    check('hash bucket collection isolated', lambda: equal(put_key('中'*2000+'a', collection=2, forced_hash=collision)[1], True))
    check('hash must be 32 bytes', lambda: rejected(lambda: db.execute('UPDATE vector_items SET item_key_hash=? WHERE id=?', (b'x'*31, first[0]))))
    check('hash must use binary binding', lambda: rejected(lambda: db.execute('UPDATE vector_items SET item_key_hash=? WHERE id=?', ('x'*32, first[0]))))
    check('collision slot cannot be negative', lambda: rejected(lambda: db.execute('UPDATE vector_items SET key_slot=-1 WHERE id=?', (first[0],))))
    nul_key = 'nul\x00key'
    nul_id, created = put_key(nul_key)
    stored_text, stored_hash = db.execute('SELECT item_key,item_key_hash FROM vector_items WHERE id=?', (nul_id,)).fetchone()
    check('vector key JSON envelope preserves NUL without raw TEXT NUL', lambda: equal(('\x00' not in stored_text, json.loads(stored_text)), (True, nul_key)))
    check('digest uses original key not JSON envelope', lambda: equal((stored_hash == hashlib.sha256(nul_key.encode('utf-8')).digest(), stored_hash != hashlib.sha256(stored_text.encode('utf-8')).digest()), (True, True)))

    result = {'result':'PASS','scope':'storage design reference fixtures and SQLite projection only','cases':cases,
              'not_tested':['product adapters','PostgreSQL/MySQL execution','concurrent collection locks','load/performance','historical migration'],
              'artifact_sha256':{p.relative_to(ROOT).as_posix():hashlib.sha256(p.read_bytes()).hexdigest()
                                 for p in (model_path,HERE/'workflow-data-nodes.json',Path(__file__).resolve())}}
    destination = ROOT/'data/maintenance/workflow-index-validation.json'
    destination.parent.mkdir(parents=True,exist_ok=True)
    evidence = destination.parent/'workflow-review-evidence'
    evidence.mkdir(exist_ok=True)
    payload = (json.dumps(result,ensure_ascii=False,indent=2)+'\n').encode('utf-8')
    for content in ([destination.read_bytes()] if destination.exists() else [])+[payload]:
        snapshot = evidence/f'indexes-{hashlib.sha256(content).hexdigest()}.json'
        if not snapshot.exists():
            snapshot.write_bytes(content)
    destination.write_bytes(payload)
    print(f'PASS: {len(cases)} storage index reference cases; {destination}')


if __name__ == '__main__':
    run()
