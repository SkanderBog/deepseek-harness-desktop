import os
import re
import shutil
import subprocess
import tempfile
from pathlib import Path

checker = Path(__file__).with_name('check-appimage-media.sh').resolve()

def plugin_library(element):
    details = subprocess.check_output(['gst-inspect-1.0', element], text=True, env={**os.environ, 'LC_ALL': 'C'})
    return Path(re.search(r'^\s*Filename\s+(.+)$', details, re.M)[1])

with tempfile.TemporaryDirectory(prefix='pet-media-test-') as directory:
    appdir = Path(directory) / 'bundle with spaces'
    plugins = appdir / 'usr/lib/gstreamer-1.0'
    plugins.mkdir(parents=True)
    scanner = appdir / 'usr/lib/gstreamer1.0/gstreamer-1.0/gst-plugin-scanner'
    scanner.parent.mkdir(parents=True)
    source = plugin_library('vp9parse')
    shutil.copy2(source.parent.parent / 'gstreamer1.0/gstreamer-1.0/gst-plugin-scanner', scanner)
    binaries = [scanner]
    for element in ['vp9parse', 'vp9alphadecodebin', 'vp9dec', 'matroskademux']:
        library = plugin_library(element)
        destination = plugins / library.name
        shutil.copy2(library, destination)
        binaries.append(destination)
    host_abi = {'libc.so.6', 'libm.so.6', 'libdl.so.2', 'libpthread.so.0', 'librt.so.1',
                'libresolv.so.2', 'libutil.so.1', 'libstdc++.so.6', 'libgcc_s.so.1',
                'libz.so.1', 'libdrm.so.2'}
    for binary in binaries:
        output = subprocess.check_output(['ldd', str(binary)], text=True)
        for name, source in re.findall(r'^\s*(\S+)\s+=>\s+(/\S+)\s+\(', output, re.M):
            if name not in host_abi:
                shutil.copy2(source, appdir / 'usr/lib' / name)
    subprocess.run(['bash', str(checker), str(appdir)], check=True)
    vpx = next((appdir / 'usr/lib').glob('libvpx.so.*'))
    vpx.unlink()
    missing_dependency = subprocess.run(['bash', str(checker), str(appdir)], capture_output=True, text=True)
    assert missing_dependency.returncode != 0 and 'libvpx' in missing_dependency.stderr, missing_dependency
    (plugins / 'libgstvideoparsersbad.so').unlink()
    missing_plugin = subprocess.run(['bash', str(checker), str(appdir)], capture_output=True, text=True)
    assert missing_plugin.returncode != 0 and 'Missing bundled media library' in missing_plugin.stderr, missing_plugin
    print('Passed: complete bundle, missing bundled dependency, missing plugin, paths with spaces')
