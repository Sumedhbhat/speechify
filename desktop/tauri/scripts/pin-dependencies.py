#!/usr/bin/env python3
import urllib.request,json,datetime,pathlib,concurrent.futures
root=pathlib.Path(__file__).resolve().parents[1]
cutoff=datetime.datetime(2026,10,11,tzinfo=datetime.timezone.utc)
crates=['tauri','tauri-build','serde','serde_json','toml','rusqlite','reqwest','sha2','libloading','hound','cpal','libc','regex','base64','tauri-plugin-single-instance']
npm=['typescript','vite','@tauri-apps/api','@tauri-apps/cli']
def get(item):
 name,kind=item
 url=f'https://crates.io/api/v1/crates/{name}' if kind=='cargo' else f'https://registry.npmjs.org/{name}'
 req=urllib.request.Request(url,headers={'User-Agent':'SpeechifyCandidate/0.1 dependency-age-check'})
 d=json.load(urllib.request.urlopen(req))
 entries=[(v['num'],v['created_at']) for v in d['versions'] if not v['yanked']] if kind=='cargo' else list(d['time'].items())
 valid=[]
 for v,date in entries:
  if len(v.split('.'))!=3 or any(not x.isdigit() for x in v.split('.')):continue
  dt=datetime.datetime.fromisoformat(date.replace('Z','+00:00'))
  if dt<=cutoff:valid.append((tuple(map(int,v.split('.'))),v,date))
 _,v,date=max(valid)
 return name,{'version':v,'published':date,'registry':url}
with concurrent.futures.ThreadPoolExecutor(max_workers=4) as p: pins=dict(p.map(get,[(c,'cargo') for c in crates]+[(n,'npm') for n in npm]))
(root/'dependency-pins.json').write_text(json.dumps({'cutoff':'2026-10-11','pins':pins},indent=2)+'\n')
pkg={'name':'speechify-tauri-candidate','private':True,'version':'0.1.0','type':'module','scripts':{'build':'tsc --noEmit && vite build','dev':'vite --host 127.0.0.1','tauri':'tauri'},'dependencies':{'@tauri-apps/api':pins['@tauri-apps/api']['version']},'devDependencies':{n:pins[n]['version'] for n in npm if n!='@tauri-apps/api'}}
(root/'package.json').write_text(json.dumps(pkg,indent=2)+'\n')
features={'tauri':['tray-icon','image-png'],'serde':['derive'],'rusqlite':['bundled','backup'],'reqwest':['blocking','json','rustls']}
lines=['[package]','name = "speechify-tauri-candidate"','version = "0.1.0"','edition = "2021"','default-run = "speechify-tauri-candidate"','[build-dependencies]',f'tauri-build = "={pins["tauri-build"]["version"]}"','[dependencies]']
for c in crates:
 if c=='tauri-build':continue
 opts=[f'version = "={pins[c]["version"]}"']
 if c in features:opts.append('features = '+json.dumps(features[c]))
 if c=='reqwest':opts.append('default-features = false')
 if c in ['tauri','cpal','tauri-plugin-single-instance']:opts.append('optional = true')
 lines.append(c+' = { '+', '.join(opts)+' }')
lines+=['[features]','default = ["desktop"]','desktop = ["dep:tauri", "dep:cpal", "dep:tauri-plugin-single-instance"]','[profile.dev]','debug = 0','incremental = false','[profile.release]','debug = 0','strip = true','lto = "thin"','codegen-units = 8']
(root/'src-tauri/Cargo.toml').write_text('\n'.join(lines)+'\n')
print(json.dumps(pins,indent=2))
