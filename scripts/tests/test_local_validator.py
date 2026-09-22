"""Exercise validator ownership/cleanup without starting Solana or building SBF."""
import os
from pathlib import Path
import shutil
import signal
import subprocess
import tempfile
import time
import unittest


REPO = Path(__file__).resolve().parents[2]


class LocalValidatorTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='manifest-validator-test-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        (self.root / 'bin').mkdir()
        (self.root / 'scripts').mkdir()
        (self.root / 'target/deploy').mkdir(parents=True)
        for name in ['manifest', 'wrapper', 'ui-wrapper']:
            (self.root / 'programs' / name).mkdir(parents=True)
            (self.root / 'target/deploy' / f'{name.replace("-", "_")}.so').touch()
        (self.root / 'test-ledger').mkdir()
        (self.root / 'test-ledger/keep').write_text('not owned by this run')
        for file in ['local-validator-test.sh', 'scripts/assert-sbpf-v3.sh']:
            shutil.copy2(REPO / file, self.root / file)
        self.env = dict(os.environ, PATH=f'{self.root / "bin"}:{os.environ["PATH"]}',
                        TMPDIR=str(self.root), TEST_FIXTURE_ROOT=str(self.root))
        self.stub('cargo', 'exit 0', shell=True)
        self.stub('readelf', "printf '  Flags: 0x3\n'", shell=True)
        self.stub('solana', '''
import os
from pathlib import Path
root = Path(os.environ['TEST_FIXTURE_ROOT'])
assert 'config' not in sys.argv, 'must not mutate user config'
sys.exit(0 if os.environ['TEST_SCENARIO'] == 'existing' or (root / 'ready').exists() else 1)
''')
        self.stub('solana-test-validator', '''
import os, time
from pathlib import Path
root = Path(os.environ['TEST_FIXTURE_ROOT'])
(root / 'pid').write_text(str(os.getpid()))
(root / 'ledger').write_text(sys.argv[sys.argv.index('--ledger') + 1])
if os.environ['TEST_SCENARIO'] == 'startup-failure':
    sys.exit(9)
if os.environ['TEST_SCENARIO'] != 'interrupt':
    (root / 'ready').touch()
while True:
    time.sleep(1)
''')
        self.stub('yarn', '''
import os
from pathlib import Path
(Path(os.environ['TEST_FIXTURE_ROOT']) / 'tested').touch()
sys.exit(7 if os.environ['TEST_SCENARIO'] == 'test-failure' else 0)
''')

    def stub(self, name, body, shell=False):
        path = self.root / 'bin' / name
        path.write_text(('#!/bin/sh\n' if shell else '#!/usr/bin/env python3\nimport sys\n') + body)
        path.chmod(0o755)

    def verify_cleanup(self):
        self.assertTrue((self.root / 'test-ledger/keep').exists())
        ledger = self.root / 'ledger'
        if ledger.exists():
            self.assertFalse(Path(ledger.read_text()).exists(), 'temporary ledger leaked')
            with self.assertRaises(ProcessLookupError):
                os.kill(int((self.root / 'pid').read_text()), 0)

    def test_success_and_failures_under_sh_and_bash(self):
        for shell in ['sh', 'bash']:
            for scenario, status in [('success', 0), ('test-failure', 7), ('startup-failure', 1), ('existing', 1)]:
                with self.subTest(shell=shell, scenario=scenario):
                    for name in ['ready', 'pid', 'ledger', 'tested']:
                        (self.root / name).unlink(missing_ok=True)
                    result = subprocess.run([shell, 'local-validator-test.sh'], cwd=self.root,
                                            env=dict(self.env, TEST_SCENARIO=scenario),
                                            capture_output=True, text=True, timeout=20)
                    self.assertEqual(result.returncode, status, result.stdout + result.stderr)
                    self.assertEqual((self.root / 'tested').exists(), scenario in ['success', 'test-failure'])
                    self.verify_cleanup()

    def test_termination_cleans_up_owned_validator(self):
        with subprocess.Popen(['sh', 'local-validator-test.sh'], cwd=self.root,
                              env=dict(self.env, TEST_SCENARIO='interrupt'),
                              stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True) as process:
            try:
                for _ in range(500):
                    if (self.root / 'pid').exists():
                        break
                    time.sleep(0.01)
                self.assertTrue((self.root / 'pid').exists())
                process.send_signal(signal.SIGTERM)
                stdout, stderr = process.communicate(timeout=20)
                self.assertEqual(process.returncode, 143, stdout + stderr)
            finally:
                if process.poll() is None:
                    process.terminate()
                    process.wait(timeout=20)
        self.verify_cleanup()


if __name__ == '__main__':
    unittest.main()
