#!/usr/bin/env python3
"""Audit locked dependencies against the fixed comparison cutoff; --fix-cargo downgrades only lock entries."""
import pathlib,json,tomllib,datetime,subprocess,sys,urllib.request,concurrent.futures
root=pathlib.Path(__file__).resolve().parents[1]
cut='2026-10-11T00:00:00Z'
cache=next((pathlib.Path.home()/'.cargo/registry/index').glob('*/.cache'))
def records(name):
 p= name if len(name)==1 else '2/'+name if len(name)==2 else '3/'+name[0]+'/'+name if len(name)==3 else name[:2]+'/'+name[2:4]+'/'+name
 return [json.loads(x) for x in (cache/p).read_bytes().split(b'\0') if x.startswith(b'{')]
issues=[];checked=[]
for pkg in tomllib.loads((root/'src-tauri/Cargo.lock').read_text())['package']:
 if not pkg.get('source','').startswith('registry'):continue
 recs=records(pkg['name']);r=next(x for x in recs if x['vers']==pkg['version']);when=r.get('pubtime')
 checked.append({'ecosystem':'cargo','name':pkg['name'],'version':pkg['version'],'published':when})
 if not when or when>cut:
  issues.append((pkg,recs))
if '--fix-cargo' in sys.argv:
 issues.sort(key=lambda item: (not item[0]['name'].startswith(('tauri','wry','tao')), item[0]['name']))
 for pkg,recs in issues:
  v=pkg['version'].split('.');prefix=v[:1] if v[0]!='0' else v[:2]
  valid=[r for r in recs if r.get('pubtime','9999')<=cut and not r['yanked'] and r['vers'].split('.')[:len(prefix)]==prefix and all(x.isdigit() for x in r['vers'].split('.'))]
  if not valid:
   print('No eligible same-series version for '+pkg['name']+'; check whether a parent downgrade removed it')
   continue
  target=max(valid,key=lambda r:tuple(map(int,r['vers'].split('.'))))['vers']
  subprocess.run(['cargo','update','-p',pkg['name']+'@'+pkg['version'],'--precise',target],cwd=root/'src-tauri',check=False)
 raise SystemExit('Rerun audit without --fix-cargo to record the final graph')
lock=json.loads((root/'package-lock.json').read_text())
items=[]
for path,pkg in lock['packages'].items():
 if not path:continue
 name=path.split('node_modules/')[-1]
 items.append((name,pkg['version']))
def npm_date(item):
 name,v=item
 data=json.load(urllib.request.urlopen('https://registry.npmjs.org/'+name))
 return {'ecosystem':'npm','name':name,'version':v,'published':data['time'].get(v)}
with concurrent.futures.ThreadPoolExecutor(max_workers=5)as pool:checked+=list(pool.map(npm_date,items))
bad=[p for p in checked if not p['published'] or p['published']>cut]
(root/'dependency-age-evidence.json').write_text(json.dumps({'cutoff':cut,'checked':checked,'too_new':bad},indent=2)+'\n')
print(f'{len(checked)} locked dependencies checked; {len(bad)} too new')
for p in bad:print(p)
sys.exit(bool(bad))
