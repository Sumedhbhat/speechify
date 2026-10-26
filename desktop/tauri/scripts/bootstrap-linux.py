#!/usr/bin/env python3
"""Extract build prerequisites locally; never install system packages."""
import subprocess,pathlib,concurrent.futures
root=pathlib.Path(__file__).resolve().parents[1]/'.build'
packages=['build-essential','pkg-config','libwebkit2gtk-4.1-dev','libgtk-3-dev','libasound2-dev','libayatana-appindicator3-dev','libxdo-dev']
sim=subprocess.check_output(['apt-get','-s','install','--no-install-recommends',*packages],text=True)
names=[l.split()[1] for l in sim.splitlines() if l.startswith('Inst ')]
(root/'debs').mkdir(parents=True,exist_ok=True)
(root/'sysroot').mkdir(exist_ok=True)
def get(name):
 subprocess.run(['apt-get','download',name],cwd=root/'debs',check=True,stdout=subprocess.DEVNULL)
with concurrent.futures.ThreadPoolExecutor(max_workers=6) as pool:list(pool.map(get,names))
for deb in (root/'debs').glob('*.deb'):subprocess.run(['dpkg-deb','-x',str(deb),str(root/'sysroot')],check=True)
print(f'Extracted {len(names)} packages to {root}/sysroot')
