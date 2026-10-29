#!/usr/bin/env python3
"""Explicit, isolated optional-model/training installation. Requires uv on PATH."""
import argparse,pathlib,subprocess,os,json,urllib.request,hashlib,shutil,sys
p=argparse.ArgumentParser();p.add_argument('family',choices=['whisper','transformers','training']);p.add_argument('--model',choices=['distil-whisper-large-v3','kyutai-stt-1b','kyutai-stt-2.6b']);p.add_argument('--data-parent',type=pathlib.Path);args=p.parse_args()
if args.family=='training' and sys.platform=='darwin':raise SystemExit('Fine-tuning requires NVIDIA CUDA on Windows or Linux')
candidate=pathlib.Path(__file__).resolve().parents[1]
source=candidate/'src-tauri/resources'
if not source.exists():source=candidate/'resources'
base=args.data_parent or (pathlib.Path(os.environ['APPDATA']) if os.name=='nt' else pathlib.Path.home()/('Library/Application Support' if __import__('sys').platform=='darwin' else '.local/share'))
root=base/'speechify-tauri-candidate';root.mkdir(parents=True,exist_ok=True)
env=root/'environments'/args.family
uv_env={**os.environ,'UV_CACHE_DIR':str(root/'cache/uv'),'UV_PYTHON_INSTALL_DIR':str(root/'environments/python')}
(env/'candidate-ready').unlink(missing_ok=True)
subprocess.run(['uv','venv','--clear','--python','3.12',str(env)],env=uv_env,check=True)
python=env/('Scripts/python.exe' if os.name=='nt' else 'bin/python')
backend=['--torch-backend','cu126'] if args.family=='training' else []
subprocess.run(['uv','pip','sync',*backend,'--python',str(python),'--require-hashes',str(source/'requirements'/f'{args.family}.lock')],env=uv_env,check=True)
(env/'candidate-ready').write_text('cuda-cu126-v1\n' if args.family=='training' else 'complete\n')
if args.model:
 if (args.model.startswith('distil') and args.family!='whisper') or (args.model.startswith('kyutai') and args.family!='transformers'):raise SystemExit('Model and environment family mismatch')
 spec=json.loads((source/'exception-models.json').read_text())[args.model]
 dest=root/'models'/args.model;partial=dest.with_name(dest.name+'.partial')
 if dest.exists():raise SystemExit('Model already exists; remove only its candidate directory before reinstalling')
 if partial.exists():shutil.rmtree(partial)
 partial.mkdir(parents=True)
 try:
  for f in spec['files']:
   rel=pathlib.PurePosixPath(f['path'])
   if rel.is_absolute() or '..' in rel.parts:raise ValueError('Unsafe model filename')
   dst=partial/rel;dst.parent.mkdir(parents=True,exist_ok=True)
   digest=hashlib.sha256() if f['sha256'] else hashlib.sha1(b'blob '+str(f['size']).encode()+b'\0')
   size=0
   with urllib.request.urlopen(f"https://huggingface.co/{spec['repo']}/resolve/{spec['revision']}/{f['path']}",timeout=30)as response,dst.open('wb')as out:
    while data:=response.read(1024*1024):
     size+=len(data)
     if size>f['size']:raise ValueError('Unexpected model size')
     out.write(data);digest.update(data);print(f"{f['path']}: {size}/{f['size']}",flush=True)
   if size!=f['size'] or digest.hexdigest()!=(f['sha256'] or f['git_blob']):raise ValueError('Model checksum mismatch: '+f['path'])
  (partial/'candidate-provenance.json').write_text(json.dumps(spec,indent=2)+'\n');partial.rename(dest)
 except BaseException:
  shutil.rmtree(partial);raise
print('Installed only into',root)
