"""Synthetic fixture-boundary tests. No download, ESM, provider or product stack."""
import hashlib
import os
import pathlib
import stat
import sys
import tempfile
import time
from unittest import mock
import unittest
import cli_auth_ci as fixture

BODY = b'\x7fELFsynthetic fixed byte fixture only'
SHA = hashlib.sha256(BODY).hexdigest()
URL = 'https://fixture.invalid/exact'

class Reply:
    def __init__(self, body=BODY, *, status=200, url=URL, length=None, clock=None):
        self.body, self.status, self.url, self.clock = body, status, url, clock
        self.headers = {'content-length': str(len(BODY)) if length is None else length}
        self.position = 0
    def __enter__(self): return self
    def __exit__(self, *unused): return None
    def geturl(self): return self.url
    def timeout(self, seconds):
        assert 0 < seconds <= 45
    def read1(self, amount):
        if self.clock is not None:
            self.clock[0] += 46
        part = self.body[self.position:self.position+amount]
        self.position += len(part)
        return part

class Boundary(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = pathlib.Path(self.temp.name)
        self.root.chmod(0o700)
        self.path = self.root/'esm'
        self.invocations = []
    def tearDown(self): self.temp.cleanup()
    def fetch(self, response=None, **kwargs):
        return fixture._fetch(self.path, url=URL, size=len(BODY), expected=SHA,
                              opener=lambda *_: response or Reply(), **kwargs)
    def accepted_call(self, response=None):
        identity = self.fetch(response)
        fixture.activate(self.path, identity, size=len(BODY), expected=SHA)
        return fixture.checked_call(self.path, identity,
                                    lambda path: self.invocations.append((path, path.read_bytes())),
                                    size=len(BODY), expected=SHA)
    def assert_refused(self, response):
        with self.assertRaises(fixture.Refusal): self.accepted_call(response)
        self.assertEqual(self.invocations, [])
        if self.path.exists(): self.assertEqual(stat.S_IMODE(self.path.stat().st_mode), 0o600)
    def test_exact_checked_path_and_bytes(self):
        self.accepted_call()
        self.assertEqual(self.invocations, [(self.path, BODY)])
        self.assertEqual(stat.S_IMODE(self.path.stat().st_mode), 0o500)
    def test_truncated_oversized_and_equal_length_wrong_hash(self):
        for body in [BODY[:-1], BODY+b'x', b'x'*len(BODY)]:
            with self.subTest(body=body):
                self.assert_refused(Reply(body))
                self.path.unlink()
    def test_false_length_and_unsuccessful_status(self):
        for response in [Reply(length='bad'), Reply(length=str(len(BODY)+1)),
                         Reply(length='0'), Reply(status=404), Reply(status=500)]:
            with self.subTest(response=response):
                self.assert_refused(response)
                self.path.unlink()
    def test_redirect_and_protocol_refuse_without_invocation(self):
        for response in [Reply(status=302), Reply(url='https://other.invalid/esm'), Reply(url='http://fixture.invalid/exact')]:
            self.assert_refused(response); self.path.unlink()
        with self.assertRaises(fixture.Refusal):
            fixture._fetch(self.path, url='http://fixture.invalid/exact', size=len(BODY), expected=SHA,
                           opener=lambda *_: self.fail('opener must not run'))
        with self.assertRaises(fixture.Refusal): fixture.open_response('https://other.invalid/esm', 1)
        self.assertFalse(self.path.exists()); self.assertEqual(self.invocations, [])
    def test_cumulative_deadline_after_dribbling_read(self):
        clock = [0.0]
        with self.assertRaises(fixture.Refusal):
            self.fetch(Reply(clock=clock), clock=lambda: clock[0])
        self.assertEqual(self.invocations, [])
        self.assertEqual(stat.S_IMODE(self.path.stat().st_mode), 0o600)
    def test_alias_hardlink_and_changed_parent(self):
        original = self.root/'original'; original.write_bytes(BODY); original.chmod(0o600)
        for link in ['symbolic','hard']:
            if link == 'symbolic': self.path.symlink_to(original)
            else: os.link(original, self.path)
            with self.assertRaises((fixture.Refusal,OSError)): self.accepted_call()
            self.assertEqual(original.read_bytes(), BODY)
            self.assertEqual(stat.S_IMODE(original.stat().st_mode), 0o600)
            self.path.unlink()
        alias = self.root/'alias'; alias.symlink_to(self.root, target_is_directory=True)
        with self.assertRaises(fixture.Refusal):
            fixture._fetch(alias/'esm',url=URL,size=len(BODY),expected=SHA,opener=lambda *_: Reply())
        self.assertEqual(self.invocations, [])
    def test_mutation_before_permission_or_execution(self):
        identity = self.fetch()
        self.path.write_bytes(b'x'*len(BODY))
        with self.assertRaises(fixture.Refusal): fixture.activate(self.path,identity,size=len(BODY),expected=SHA)
        self.assertEqual(stat.S_IMODE(self.path.stat().st_mode),0o600)
        self.path.unlink(); identity = self.fetch()
        fixture.activate(self.path,identity,size=len(BODY),expected=SHA)
        self.path.chmod(0o600); self.path.write_bytes(b'x'*len(BODY)); self.path.chmod(0o500)
        with self.assertRaises(fixture.Refusal):
            fixture.checked_call(self.path,identity,lambda _: self.invocations.append('bad'),size=len(BODY),expected=SHA)
        self.assertEqual(self.invocations, [])
    def test_same_bytes_replacement_identity_refuses(self):
        identity = self.fetch()
        other = self.root/'replacement';other.write_bytes(BODY);other.chmod(0o600);other.replace(self.path)
        with self.assertRaises(fixture.Refusal): fixture.activate(self.path,identity,size=len(BODY),expected=SHA)
        self.assertEqual(self.invocations, [])
    def test_production_url_and_trust_constants(self):
        self.assertEqual(fixture.SIZE,10331544)
        self.assertEqual(fixture.DIGEST,'b593e8d2d216515a63b140d85bf3e0276e5a08c970b62a398edcd0b718ffda2b')
        self.assertEqual(fixture.URL,'https://install.enscrive.io/releases/dev/v20260829-1622/x86_64-unknown-linux-gnu/esm')
        class BadHeaders:
            status = 200
            def getheaders(self): return [('Content-Length','1'),('Content-Length','2')]
        with self.assertRaises(fixture.Refusal): fixture.Response(None,BadHeaders(),None)
    def test_capture_success_overflow_timeout_and_inherited_pipe(self):
        env={'PATH':'/usr/bin:/bin','HOME':str(self.root),'TMPDIR':str(self.root),'LANG':'C.UTF-8'}
        self.assertEqual(fixture.capture([sys.executable,'-c',"print('ok')"],self.root,env,seconds=3),b'ok\n')
        for code in ["print('x'*10000)","import time;time.sleep(10)",
                     "import os,time;time.sleep(10) if os.fork()==0 else None"]:
            with self.subTest(code=code):
                with self.assertRaises(fixture.Refusal): fixture.capture([sys.executable,'-c',code],self.root,env,seconds=3,cap=128)

    def test_worker_bounds_opener_headers_and_readback_stalls(self):
        module_dir = str(pathlib.Path(fixture.__file__).resolve().parent)
        for phase in ['opener','headers','readback','success']:
            script = self.root/('worker-'+phase+'.py')
            script.write_text(
                'import sys,time,pathlib\n'
                + 'sys.path.insert(0,'+repr(module_dir)+')\nimport cli_auth_ci as f\n'
                + 'body='+repr(BODY)+'\nphase='+repr(phase)+'\n'
                + "class Reply:\n status=200\n headers={}\n def __enter__(self):return self\n def __exit__(self,*a):pass\n def geturl(self):\n  if phase=='headers':time.sleep(10)\n  return 'https://fixture.invalid/exact'\n def timeout(self,t):pass\n def read1(self,n):\n  global body\n  part=body[:n];body=body[n:];return part\n"
                + "def opener(*a):\n if phase=='opener':time.sleep(10)\n return Reply()\n"
                + "original=f.identity\ndef identity(*a):\n if phase=='readback':time.sleep(10)\n return original(*a)\nf.identity=identity\n"
                + 'f._fetch(pathlib.Path(sys.argv[1]),url='+repr(URL)+',size='+str(len(BODY))+',expected='+repr(SHA)+',opener=opener)\n'
            )
            started = time.monotonic()
            command = [sys.executable,'-B',str(script),str(self.path)]
            if phase == 'success':
                checked = fixture._worker_fetch(self.path,command,size=len(BODY),expected=SHA,seconds=3)
                fixture.activate(self.path,checked,size=len(BODY),expected=SHA)
                fixture.checked_call(self.path,checked,lambda p:self.invocations.append((p,p.read_bytes())),size=len(BODY),expected=SHA)
                self.assertEqual(self.invocations,[(self.path,BODY)])
            else:
                with self.assertRaises(fixture.Refusal): fixture._worker_fetch(self.path,command,size=len(BODY),expected=SHA,seconds=3)
                self.assertEqual(self.invocations,[])
                if self.path.exists(): self.assertEqual(stat.S_IMODE(self.path.stat().st_mode),0o600)
            self.assertLess(time.monotonic()-started,3.5)
            if self.path.exists(): self.path.unlink()

    def test_expired_ready_final_download_readback(self):
        clock = [0.0]
        original = fixture.identity
        def expired(*args):
            value = original(*args)
            clock[0] = 46
            return value
        with mock.patch.object(fixture,'identity',side_effect=expired):
            with self.assertRaises(fixture.Refusal): self.fetch(clock=lambda:clock[0])
        self.assertEqual(self.invocations,[])
        self.assertEqual(stat.S_IMODE(self.path.stat().st_mode),0o600)

    def test_success_leader_closed_stdio_descendant_is_not_left_running(self):
        pidfile = self.root/'descendant.pid'
        code = "import os,time,pathlib;child=os.fork()\nif child==0:\n fd=os.open('/dev/null',os.O_RDWR)\n for n in (0,1,2):os.dup2(fd,n)\n time.sleep(10)\nelse:\n pathlib.Path("+repr(str(pidfile))+").write_text(str(child))\n print('ok')\n"
        env={'PATH':'/usr/bin:/bin','HOME':str(self.root),'TMPDIR':str(self.root),'LANG':'C.UTF-8'}
        self.assertEqual(fixture.capture([sys.executable,'-c',code],self.root,env,seconds=3),b'ok\n')
        child = int(pidfile.read_text())
        try: fields=(pathlib.Path('/proc')/str(child)/'stat').read_text().rsplit(')',1)[1].split()
        except FileNotFoundError: fields=['X']
        self.assertIn(fields[0],('Z','X'))

    def test_expired_ready_capture_refuses(self):
        marker=self.root/'ready'
        # Child has closed stdout and exited before the clock is advanced by the
        # deterministic acceptance hook, not merely a sleeping-process timeout.
        original_group=fixture.live_group
        crossed=[False]
        def group(group_id):
            value=original_group(group_id)
            if not value:crossed[0]=True
            return value
        clock=lambda:time.monotonic()+(100 if crossed[0] else 0)
        env={'PATH':'/usr/bin:/bin','HOME':str(self.root),'TMPDIR':str(self.root),'LANG':'C.UTF-8'}
        with mock.patch.object(fixture,'live_group',side_effect=group):
            with self.assertRaises(fixture.Refusal): fixture.capture([sys.executable,'-c',"print('ready')"],self.root,env,seconds=3,_clock=clock)
        self.assertTrue(crossed[0])

    def test_cleanup_error_preserves_captured_failure_output(self):
        env={'PATH':'/usr/bin:/bin','HOME':str(self.root),'TMPDIR':str(self.root),'LANG':'C.UTF-8'}
        with mock.patch.object(fixture,'live_group',side_effect=fixture.Refusal('synthetic-probe-failure')):
            with self.assertRaises(fixture.Refusal) as caught:
                fixture.capture([sys.executable,'-c',"import sys;print('bounded witness');sys.exit(2)"],self.root,env,seconds=3)
        self.assertEqual(str(caught.exception),'cleanup-unconfirmed')
        self.assertEqual(caught.exception.previous_kind,'command-failed')
        self.assertEqual(caught.exception.output,b'bounded witness\n')

    def test_postspawn_selector_setup_and_close_failures_still_reap(self):
        environment={'PATH':'/usr/bin:/bin','HOME':str(self.root),'TMPDIR':str(self.root),'LANG':'C.UTF-8'}
        original_popen = fixture.subprocess.Popen
        for stage in ['create','register-and-close']:
            children=[]
            def launch(*args,**kwargs):
                child=original_popen(*args,**kwargs);children.append(child);return child
            class BrokenSelector:
                def register(self,*args): raise OSError('synthetic register failure')
                def close(self): raise OSError('synthetic close failure')
            selector = mock.Mock(side_effect=OSError('synthetic create failure')) if stage=='create' else BrokenSelector
            with mock.patch.object(fixture.subprocess,'Popen',side_effect=launch), mock.patch.object(fixture.selectors,'DefaultSelector',selector):
                with self.assertRaises(fixture.Refusal) as caught:
                    fixture.capture([sys.executable,'-c','import time;time.sleep(10)'],self.root,environment,seconds=3)
            self.assertEqual(len(children),1)
            self.assertIsNotNone(children[0].returncode)
            self.assertEqual(fixture.live_group(children[0].pid),[])
            self.assertEqual(str(caught.exception),'command-unavailable')
            self.assertEqual(caught.exception.output,b'')

    def test_durable_failure_record_preserves_only_static_previous_kind(self):
        error=fixture.Refusal('cleanup-unconfirmed')
        error.previous_kind='command-failed'
        record=fixture.failure_record(error)
        path=self.root/'failure.json'
        fixture.write_json(path,record)
        import json
        self.assertEqual(json.loads(path.read_text()),{'passed':False,'kind':'cleanup-unconfirmed','previous_kind':'command-failed'})
        error.previous_kind='synthetic-not-a-classification'
        self.assertIsNone(fixture.failure_record(error)['previous_kind'])

if __name__ == '__main__': unittest.main()
