"""Derive the Agent operation directory from the actual workspace route registrations."""
import json,re,sys
from pathlib import Path
from functools import lru_cache

root=Path(__file__).resolve().parents[1]
source=root/'crates/server/src'
operations={}

@lru_cache(maxsize=128)
def code_only(text):
    tokens=r'''r(#+)"[\s\S]*?"\1|r"[^"]*"|"(?:\\.|[^"\\])*"|'(?:\\.|[^'\\\r\n])'|//[^\n]*|/\*[\s\S]*?\*/'''
    return re.sub(tokens,lambda m:' '*len(m[0]),text)

def block(text,start):
    """Return one brace-delimited source block, ignoring quoted string braces."""
    depth=0;code=code_only(text)
    for end in range(start,len(text)):
        ch=code[end]
        if ch=='{':depth+=1
        elif ch=='}':
            depth-=1
            if depth==0:return text[start:end+1]
    return ''

def expand_module(text,module):
    # Expand only the project's single-arm resource macros, from their real arguments.
    for macro in re.finditer(r'macro_rules!\s+(\w+)\s*\{\s*\((.*?)\)\s*=>\s*\{',text,re.S):
        params=re.findall(r'\$(\w+)\s*:',macro[2])
        for invocation in re.finditer(r'\b'+macro[1]+r'!\(\s*'+re.escape(module)+r'\s*,([^;]*?)\)\s*;',text,re.S):
            arguments=[module]+[s.strip() for s in invocation[1].strip().rstrip(',').split(',')]
            if len(params)!=len(arguments):continue
            expanded=block(text,macro.end()-1)
            for param,value in zip(params,arguments):expanded=expanded.replace('$'+param,value)
            expanded=re.sub(r'concat!\(\s*"([^"]*)"\s*,\s*"([^"]*)"\s*\)',lambda m:json.dumps(m[1]+m[2]),expanded)
            return expanded
    return None
def declaration(text,name,kind='struct'):
    match=re.search(r'\b'+kind+r'\s+'+re.escape(name)+r'\b[^\{]*\{',text)
    if not match:return None
    depth=1
    for end in range(match.end(),len(text)):
        if text[end]=='{':depth+=1
        elif text[end]=='}':
            depth-=1
            if depth==0:return text[match.start():end+1]
    return None

def contract(handler,file):
    *modules,name=handler.split('::')
    candidates=[file]
    if modules:
        base=source.joinpath(*modules)
        candidates=[base.with_suffix('.rs'),base/'mod.rs']
        for relative in [file.parent.joinpath(*modules),file.parent/file.stem/Path(*modules)]:
            candidates.extend([relative.with_suffix('.rs'),relative/'mod.rs'])
        if base.is_dir():candidates+=sorted(p for p in base.rglob('*.rs') if 'tests' not in p.parts and p.name!='tests.rs')
        parent=base.parent
        candidates += [parent.with_suffix('.rs'),parent/'mod.rs']
    for candidate in candidates:
        if not candidate.exists():continue
        text=candidate.read_text(encoding='utf8')
        context=text
        if modules and candidate in [base.parent.with_suffix('.rs'),base.parent/'mod.rs']:
            expanded=expand_module(text,modules[-1])
            if expanded is None:continue
            text=expanded
        match=re.search(r'\b(?:async\s+)?fn\s+'+re.escape(name)+r'\s*\(',text)
        if not match:continue
        end=text.find('{',match.end())
        signature=text[match.start():end].strip()
        docs=[]
        for line in reversed(text[:match.start()].rsplit('\n',1)[0].splitlines()):
            if not line.lstrip().startswith('///'):break
            docs.insert(0,line.lstrip()[3:].strip())
        definitions=[]
        for typename in re.findall(r'(?:Json|Query)<([\w]+)>',signature):
            if typename in ['Value','HashMap']:continue
            pending=[typename];seen=set()
            while pending and len(seen)<12:
                typename=pending.pop(0)
                if typename in seen:continue
                seen.add(typename)
                definition=declaration(text,typename) or declaration(text,typename,'enum') or declaration(context,typename) or declaration(context,typename,'enum')
                if definition:
                    definitions.append(definition)
                    pending.extend(re.findall(r'\b[A-Z][A-Za-z0-9]+\b',definition))
        body=block(text,end)
        permission=re.match(r'\{\s*scope\.require\("([^"]+)"\)\?;',body)
        # Indirect checks remain explicitly described as candidates, not an invented policy.
        checks=set(re.findall(r'scope\.require\("([^"]+)"\)',body))
        known=set(re.findall(r'"([a-z][a-z-]*\.[a-z_]+)"',(source/'accounts/workspace.rs').read_text(encoding='utf8')))
        checks.update(value for value in re.findall(r'"([a-z][a-z-]*\.[a-z_]+)"',body) if value in known)
        seen={name};pending=re.findall(r'\b([a-z_]\w*)\(',body)
        for helper in pending:
            if helper in seen:continue
            seen.add(helper)
            found=re.search(r'\b(?:async\s+)?fn\s+'+re.escape(helper)+r'\s*\(',text)
            if found:
                helper_body=block(text,text.find('{',found.end()))
                checks.update(re.findall(r'scope\.require\("([^"]+)"\)',helper_body))
                checks.update(value for value in re.findall(r'"([a-z][a-z-]*\.[a-z_]+)"',helper_body) if value in known)
        return {'signature':signature,'definitions':definitions,'permission_checks':sorted(checks),**({'documentation':'\n'.join(docs)} if docs else {}),**({'permission':permission.group(1)} if permission else {})}
    return {}
for file in source.rglob('*.rs'):
    if 'tests' in file.parts or file.name=='platform_tools.rs': continue
    text=file.read_text(encoding='utf8')
    if file.name=='main.rs': text=text[text.index('fn workspace_routes()'):]
    for match in re.finditer(r'\.route\(\s*"(/\{workspace\}[^"\n]*)"\s*,',text):
        start=match.end();depth=1;quoted=False;escaped=False
        for end in range(start,len(text)):
            ch=text[end]
            if quoted:
                if escaped: escaped=False
                elif ch=='\\': escaped=True
                elif ch=='"': quoted=False
            elif ch=='"': quoted=True
            elif ch=='(': depth+=1
            elif ch==')':
                depth-=1
                if depth==0: break
        route=match.group(1).removeprefix('/{workspace}') or '/'
        for method,handler in re.findall(r'\b(get|post|put|patch|delete|any)\(\s*([\w:]+)\s*\)',text[start:end]):
            operations[(route,method)]={'path':route,'method':method.upper(),'handler':handler,'source':str(file.relative_to(root)).replace('\\','/'),**contract(handler,file)}
output=json.dumps(sorted(operations.values(),key=lambda v:(v['path'],v['method'])),ensure_ascii=False,indent=2)+'\n'
target=source/'platform_tools_catalog.json'
if '--check' in sys.argv:
    assert target.read_text(encoding='utf8')==output,'Run python scripts/generate-platform-tools.py after changing workspace routes'
else: target.write_text(output,encoding='utf8',newline='\n')
print(f'{len(operations)} workspace operations in Agent tool directory')
