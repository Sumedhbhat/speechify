#!/usr/bin/env python3
"""Offline installer failure/retry tests. No dependency installs or network."""
import hashlib, importlib.util, io, json, pathlib, tempfile, unittest, sys
sys.dont_write_bytecode = True
from unittest.mock import patch
script = pathlib.Path(__file__).resolve().parents[1] / 'src-tauri/resources/install-exception.py'
spec = importlib.util.spec_from_file_location('installer', script)
installer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(installer)

class InstallerTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = pathlib.Path(self.tmp.name)
        self.source = self.root / 'resources'
        self.source.mkdir()
        self.body = b'pinned model contents'
        self.manifest = {'distil-whisper-large-v3': {'repo': 'fixture/model', 'revision': 'pinned', 'files': [{'path': 'model.bin', 'size': len(self.body), 'sha256': hashlib.sha256(self.body).hexdigest(), 'git_blob': None}]}}
        (self.source / 'exception-models.json').write_text(json.dumps(self.manifest))
        self.file_patch = patch.object(installer, '__file__', str(self.source / 'install-exception.py'))
        self.file_patch.start()
        self.addCleanup(self.file_patch.stop)
        self.addCleanup(self.tmp.cleanup)
    def fake_run(self, args, **kwargs):
        self.assertEqual(kwargs["env"]["UV_PYTHON_INSTALL_DIR"], str(self.root / "environments/python"))
        if args[1] == 'venv': pathlib.Path(args[-1]).mkdir(parents=True, exist_ok=True)
    def install(self, payload=None):
        with patch.object(installer.subprocess, 'run', side_effect=self.fake_run), patch.object(installer.urllib.request, 'urlopen', return_value=io.BytesIO(self.body if payload is None else payload)), patch.object(installer, 'emit'):
            installer.install(self.root, 'whisper', 'distil-whisper-large-v3')
    def test_verified_completion_and_retry_after_interruption(self):
        partial = self.root / 'models/distil-whisper-large-v3.partial'
        partial.mkdir(parents=True)
        (partial / 'unverified').write_bytes(b'stale')
        self.install()
        self.assertFalse(partial.exists())
        dest = self.root / 'models/distil-whisper-large-v3'
        self.assertEqual((dest / 'model.bin').read_bytes(), self.body)
        self.assertTrue((dest / 'candidate-provenance.json').exists())
        self.assertFalse((dest / 'unverified').exists())
        self.assertTrue((self.root / 'environments/whisper/candidate-ready').exists())
        # Completed installation does not recreate the environment or fetch model files.
        with patch.object(installer.subprocess, 'run', side_effect=AssertionError('unexpected environment mutation')), patch.object(installer.urllib.request, 'urlopen', side_effect=AssertionError('unexpected network')):
            installer.install(self.root, 'whisper', 'distil-whisper-large-v3')
    def test_hash_failure_never_commits_and_retry_recovers(self):
        with self.assertRaises(ValueError): self.install(b'x' * len(self.body))
        self.assertFalse((self.root / 'models/distil-whisper-large-v3').exists())
        self.assertFalse((self.root / 'models/distil-whisper-large-v3.partial').exists())
        self.install()
        self.assertTrue((self.root / 'models/distil-whisper-large-v3/candidate-provenance.json').exists())
    def test_dependency_failure_leaves_no_ready_marker(self):
        def fail(args, **kwargs):
            self.fake_run(args, **kwargs)
            if args[1] == 'pip': raise RuntimeError('disk full')
        with patch.object(installer.subprocess, 'run', side_effect=fail), patch.object(installer, 'emit'):
            with self.assertRaises(RuntimeError): installer.install(self.root, 'whisper')
    def test_training_repairs_cpu_receipt_and_selects_cuda_backend(self):
        marker = self.root / 'environments/training/candidate-ready'
        marker.parent.mkdir(parents=True)
        marker.write_text('complete\n')
        commands = []
        def run(args, **kwargs):
            commands.append(args)
            self.fake_run(args, **kwargs)
        with patch.object(installer.sys, 'platform', 'win32'), patch.object(installer.subprocess, 'run', side_effect=run), patch.object(installer, 'emit'):
            installer.install(self.root, 'training')
        self.assertEqual(marker.read_text(), 'cuda-cu126-v1\n')
        sync = next(c for c in commands if c[1] == 'pip')
        self.assertIn('--torch-backend', sync)
        self.assertEqual(sync[sync.index('--torch-backend')+1], 'cu126')
        with patch.object(installer.sys, 'platform', 'darwin'):
            with self.assertRaisesRegex(ValueError, 'macOS training is unavailable'):
                installer.install(self.root, 'training')
        self.assertFalse((self.root / 'environments/whisper/candidate-ready').exists())
    def test_oversized_download_rejected(self):
        with self.assertRaises(ValueError): self.install(self.body + b'bad')
        self.assertFalse((self.root / 'models/distil-whisper-large-v3').exists())

if __name__ == '__main__': unittest.main(verbosity=2)
