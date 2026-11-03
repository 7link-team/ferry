import fnmatch, json, os, subprocess

c = json.loads(os.environ['REVIEW_CONFIG'])
base, head = os.environ['REVIEW_BASE'], os.environ['REVIEW_HEAD']
full = os.environ['REVIEW_FULL'] == 'true'

def git(*args):
    return subprocess.check_output(['git', *args], stderr=subprocess.DEVNULL)

# Resolve supplied SHA objects, never a moving branch or executable input.
for sha in (base, head):
    if len(sha) not in (40, 64) or any(x not in '0123456789abcdef' for x in sha):
        raise ValueError('invalid immutable SHA')
    if git('rev-parse', '--verify', sha + '^{commit}').decode().strip() != sha:
        raise ValueError('commit mismatch')

paths = git('ls-tree', '-rz', '--name-only', head) if full else git('diff', '--name-only', '-z', base, head)
files, skipped, used = [], [], 0
for raw in paths.split(b'\0'):
    if not raw:
        continue
    path = raw.decode('utf-8', errors='replace')
    reason = ''
    if c['include'] and not any(fnmatch.fnmatchcase(path, p) for p in c['include']):
        reason = 'outside_include'
    if any(fnmatch.fnmatchcase(path, p) for p in c['exclude']):
        reason = 'excluded'
    if len(files) >= c['max_files']:
        reason = 'file_budget'
    if reason:
        skipped.append({'file': path, 'reason': reason})
        continue
    try:
        # Read objects, not the checked out filesystem or symlink targets.
        object_size = int(git('cat-file', '-s', head + ':' + path))
        if object_size > c['max_bytes'] - used:
            skipped.append({'file': path, 'reason': 'byte_budget'})
            continue
        if not full:
            try:
                prior_size = int(git('cat-file', '-s', base + ':' + path))
            except subprocess.CalledProcessError:
                prior_size = 0
            if prior_size > c['max_bytes']:
                skipped.append({'file': path, 'reason': 'byte_budget'})
                continue
        data = git('show', head + ':' + path)
        content = data.decode('utf-8')
        if '\0' in content:
            raise UnicodeError()
    except subprocess.CalledProcessError:
        skipped.append({'file': path, 'reason': 'deleted'})
        continue
    except UnicodeError:
        skipped.append({'file': path, 'reason': 'binary'})
        continue
    diff = '' if full else git('diff', '--no-ext-diff', '--no-textconv', '--unified=8', base, head, '--', path).decode('utf-8', errors='replace')
    cost = len(data) + len(diff.encode('utf-8'))
    if used + cost > c['max_bytes']:
        skipped.append({'file': path, 'reason': 'byte_budget'})
        continue
    used += cost
    files.append({'file': path, 'content': content, 'diff': diff, 'lines': len(content.splitlines())})
with open('review-input.json', 'w', encoding='utf-8') as out:
    json.dump({'base_sha': base, 'head_sha': head, 'files': files, 'skipped': skipped}, out, ensure_ascii=False)
