#!/usr/bin/env python3
"""Package the actual host release binary; never rename it as another OS."""
import pathlib,platform,shutil,hashlib,tarfile,json
root=pathlib.Path(__file__).resolve().parents[1]
name=f'speechify-tauri-candidate-{platform.system().lower()}-{platform.machine()}'
artifacts=root/'artifacts';stage=artifacts/name
if stage.exists():shutil.rmtree(stage)
stage.mkdir(parents=True,exist_ok=True)
binary='speechify-tauri-candidate'+('.exe' if platform.system()=='Windows' else '')
shutil.copy2(root/'src-tauri/target/release'/binary,stage/binary)
shutil.copytree(root/'src-tauri/resources',stage/'resources',dirs_exist_ok=True,ignore=shutil.ignore_patterns('__pycache__','*.pyc'))
shutil.copy2(root/'README.md',stage/'README.md')
shutil.copytree(root/'scripts',stage/'scripts',dirs_exist_ok=True,ignore=shutil.ignore_patterns('__pycache__','*.pyc'))
manifest=[]
for p in sorted(stage.rglob('*')):
 if p.is_file():manifest.append(hashlib.sha256(p.read_bytes()).hexdigest()+'  '+str(p.relative_to(stage)))
(stage/'SHA256SUMS').write_text('\n'.join(manifest)+'\n')
archive=artifacts/(name+'.tar.gz')
with tarfile.open(archive,'w:gz')as tar:tar.add(stage,arcname=name)
print(archive)
