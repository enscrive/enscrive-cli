#!/usr/bin/env python3
"""Fixed-byte ESM compatibility fixture. No live service or provider credentials."""
import argparse
import hashlib
import http.client
import ssl
import json
import os
import pathlib
import re
import selectors
import shutil
import signal
import stat
import sys
import subprocess
import tempfile
import time
import urllib.parse

URL = 'https://install.enscrive.io/releases/dev/v20260829-1622/x86_64-unknown-linux-gnu/esm'
SIZE = 10331544
DIGEST = 'b593e8d2d216515a63b140d85bf3e0276e5a08c970b62a398edcd0b718ffda2b'
ACTUAL = 'local::auth_tests::ens5903_actual_esm_parser_and_synthesis_preserves_auth'
ANSI = re.compile(r'\x1b\[[0-?]*[ -/]*[@-~]')

class Refusal(Exception):
    """Only static, non-secret classifications are raised by the fixture boundary."""

def require(ok, kind):
    if not ok:
        raise Refusal(kind)

def digest(path):
    h = hashlib.sha256()
    with path.open('rb') as f:
        for part in iter(lambda: f.read(65536), b''):
            h.update(part)
    return h.hexdigest()

def owned(path, directory=False):
    require(path.is_absolute(), 'relative-path')
    for item in [*reversed(path.parents), path]:
        require(not item.is_symlink(), 'symlink-path')
    s = path.lstat()
    require(s.st_uid == os.getuid(), 'foreign-owner')
    require(stat.S_ISDIR(s.st_mode) if directory else stat.S_ISREG(s.st_mode), 'wrong-file-kind')
    if not directory:
        require(s.st_nlink == 1, 'hardlink-path')
    return s

def identity(path, size, expected, mode):
    before = owned(path)
    require(stat.S_IMODE(before.st_mode) == mode and before.st_size == size, 'file-mode-or-size')
    flags = os.O_RDONLY | os.O_NOFOLLOW
    fd = os.open(path, flags)
    try:
        opened = os.fstat(fd)
        require((opened.st_dev, opened.st_ino) == (before.st_dev, before.st_ino), 'identity-changed')
        h = hashlib.sha256()
        count = 0
        while True:
            part = os.read(fd, min(65536, size + 1 - count))
            if not part:
                break
            count += len(part)
            require(count <= size, 'file-overflow')
            h.update(part)
        after = owned(path)
        require((before.st_dev, before.st_ino, before.st_size, before.st_mtime_ns, before.st_ctime_ns) ==
                (after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns, after.st_ctime_ns), 'identity-changed')
        require(count == size and h.hexdigest() == expected, 'file-digest')
        return (after.st_dev, after.st_ino)
    finally:
        os.close(fd)

class Response:
    def __init__(self, connection, response, sock):
        self.connection, self.response, self.sock = connection, response, sock
        self.status = response.status
        self.headers = {}
        for key, value in response.getheaders():
            key = key.lower()
            require(key not in self.headers or key not in ('content-length', 'content-encoding'), 'duplicate-http-framing')
            self.headers[key] = value
    def geturl(self): return URL
    def read1(self, amount): return self.response.read1(amount)
    def timeout(self, seconds): self.sock.settimeout(seconds)
    def __enter__(self): return self
    def __exit__(self, *unused):
        self.response.close()
        self.connection.close()

def open_response(url, remaining):
    require(url == URL and urllib.parse.urlsplit(url).scheme == 'https', 'unsupported-url')
    address = urllib.parse.urlsplit(url)
    # Explicit HTTPSConnection ignores proxies and never follows redirects.
    connection = http.client.HTTPSConnection(address.hostname, 443, timeout=remaining,
                                             context=ssl.create_default_context())
    try:
        deadline = time.monotonic() + remaining
        connection.connect()
        require(time.monotonic() < deadline, 'download-deadline')
        sock = connection.sock
        sock.settimeout(max(.001, deadline - time.monotonic()))
        connection.request('GET', address.path, headers={'Accept-Encoding': 'identity'})
        require(time.monotonic() < deadline, 'download-deadline')
        sock.settimeout(max(.001, deadline - time.monotonic()))
        response = connection.getresponse()
        return Response(connection, response, sock)
    except Exception:
        connection.close()
        raise

# Private injectable seam is used only by synthetic tests; production uses fixed constants.
def _fetch(path, *, url, size, expected, opener, clock=time.monotonic, seconds=45):
    owned(path.parent, True)
    require(stat.S_IMODE(path.parent.stat().st_mode) == 0o700, 'directory-mode')
    require(url.startswith('https://'), 'unsupported-protocol')
    deadline = clock() + seconds
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    try:
        os.fchmod(fd, 0o600)
        with opener(url, max(0.001, deadline - clock())) as response:
            require(clock() < deadline, 'download-deadline')
            require(response.status == 200 and response.geturl() == url, 'http-status-or-redirect')
            length = response.headers.get('content-length')
            if length is not None:
                require(length.isascii() and length.isdigit() and int(length) == size, 'content-length')
            require(response.headers.get('content-encoding', 'identity') == 'identity', 'content-encoding')
            count = 0
            h = hashlib.sha256()
            while True:
                require(clock() < deadline, 'download-deadline')
                # HTTPResponse.read1 performs at most one underlying buffered read.
                response.timeout(max(0.001, deadline - clock()))
                part = response.read1(min(65536, size + 1 - count))
                require(clock() < deadline, 'download-deadline')
                if not part:
                    break
                count += len(part)
                require(count <= size, 'download-overflow')
                h.update(part)
                view = memoryview(part)
                while view:
                    written = os.write(fd, view)
                    require(written > 0, 'download-write')
                    view = view[written:]
            require(count == size and h.hexdigest() == expected, 'download-size-or-digest')
            os.fsync(fd)
    except Refusal:
        raise
    except Exception:
        raise Refusal('download-unavailable') from None
    finally:
        os.close(fd)
    checked = identity(path, size, expected, 0o600)
    require(clock() < deadline, 'download-deadline')
    return checked

def _worker_fetch(path, command, *, size=SIZE, expected=DIGEST, seconds=45):
    deadline = time.monotonic() + seconds
    parent = owned(path.parent, True)
    require(stat.S_IMODE(parent.st_mode) == 0o700 and not path.exists() and not path.is_symlink(), 'download-path')
    environment = {'PATH':'/usr/bin:/bin', 'HOME':str(path.parent), 'TMPDIR':str(path.parent), 'LANG':'C.UTF-8'}
    # Parent capture bounds all worker phases, including DNS, header parsing,
    # fsync and worker readback. No network operation runs in this parent.
    capture(command, path.parent, environment, seconds=seconds, cap=8192)
    checked = identity(path, size, expected, 0o600)
    after = owned(path.parent, True)
    require((parent.st_dev,parent.st_ino) == (after.st_dev,after.st_ino), 'download-parent-changed')
    require(time.monotonic() < deadline, 'download-deadline')
    return checked

def fetch(path):
    return _worker_fetch(path, [sys.executable, str(pathlib.Path(__file__).resolve()),
                               'download-worker', '--destination', str(path)])

def activate(path, expected_identity, *, size=SIZE, expected=DIGEST):
    require(identity(path, size, expected, 0o600) == expected_identity, 'activation-identity')
    os.chmod(path, 0o500, follow_symlinks=False)
    require(identity(path, size, expected, 0o500) == expected_identity, 'activation-identity')

def checked_call(path, expected_identity, invoke, *, size=SIZE, expected=DIGEST):
    require(identity(path, size, expected, 0o500) == expected_identity, 'preexecute-identity')
    result = invoke(path)
    require(identity(path, size, expected, 0o500) == expected_identity, 'postexecute-identity')
    return result

def live_group(group):
    """Linux hosted CI: zombies are not running descendants; leader is reaped separately."""
    live = []
    for entry in pathlib.Path('/proc').iterdir():
        if not entry.name.isdigit():
            continue
        try:
            fields = (entry/'stat').read_text().rsplit(')',1)[1].split()
        except (FileNotFoundError, ProcessLookupError):
            continue
        except (OSError, IndexError):
            raise Refusal('cleanup-membership-unavailable') from None
        if int(fields[2]) == group and fields[0] not in ('Z','X'):
            live.append(int(entry.name))
    return live

def capture(args, cwd, env, seconds=600, cap=8*1024*1024, *, _clock=time.monotonic):
    deadline = _clock() + seconds
    buf = bytearray()
    reader = None
    error = None
    process = subprocess.Popen(args, cwd=cwd, env=env, stdin=subprocess.DEVNULL,
                               stdout=subprocess.PIPE, stderr=subprocess.STDOUT, start_new_session=True)
    try:
        reader = selectors.DefaultSelector()
        reader.register(process.stdout, selectors.EVENT_READ)
        while reader.get_map():
            require(_clock() < deadline - 2, 'command-deadline')
            for key, _ in reader.select(min(.1, max(0, deadline - 2 - _clock()))):
                part = os.read(key.fd, min(65536, cap + 1 - len(buf)))
                if not part:
                    reader.unregister(key.fileobj)
                    continue
                buf.extend(part)
                require(len(buf) <= cap, 'command-output-overflow')
        while process.poll() is None and _clock() < deadline - 2:
            time.sleep(.01)
        require(process.poll() is not None, 'command-deadline')
        require(_clock() < deadline - 2, 'command-deadline')
        require(process.returncode == 0, 'command-failed')
    except Exception as caught:
        error = caught if isinstance(caught, Refusal) else Refusal('command-unavailable')
    finally:
        # Failed setup/close must never bypass the independently attempted group
        # cleanup. Retain the original classified failure if there already is one.
        for resource in (reader, process.stdout):
            if resource is not None:
                try:
                    resource.close()
                except Exception:
                    if error is None:
                        error = Refusal('capture-close-unavailable')
        # Always clean the owned group, even when a successful leader closed its
        # streams after starting a detached-stdio descendant in this same group.
        try:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            while _clock() < deadline:
                if process.poll() is not None and not live_group(process.pid):
                    break
                time.sleep(.01)
            require(process.poll() is not None, 'cleanup-reap-unconfirmed')
            require(not live_group(process.pid), 'cleanup-live-group-remains')
        except Exception:
            cleanup = Refusal('cleanup-unconfirmed')
            cleanup.previous_kind = str(error) if error is not None else None
            error = cleanup
    if error is None and _clock() >= deadline:
        error = Refusal('command-deadline')
    if error is not None:
        error.output = bytes(buf)
        raise error
    return bytes(buf)

def failure_record(error):
    kind = str(error) if isinstance(error, Refusal) else 'fixture-unavailable'
    previous = getattr(error, 'previous_kind', None)
    if previous not in {'command-failed', 'command-deadline', 'command-unavailable',
                        'command-output-overflow', 'capture-close-unavailable'}:
        previous = None
    return {'passed': False, 'kind': kind, 'previous_kind': previous}

def write_json(path, value):
    path.write_text(json.dumps(value, indent=2) + '\n')

def ordinary(repo, evidence):
    # Same existing required suite/arguments/environment; capture makes its actual
    # selected names reviewable. This is not the private ESM fixture environment.
    data = capture(['cargo', 'test', '--locked'], repo, dict(os.environ), seconds=1200, cap=32*1024*1024)
    (evidence/'ordinary-tests.log').write_bytes(data)

def fixture(repo, evidence):
    scripts = repo/'scripts'
    manifest = json.loads((scripts/'cli_auth_esm_fixture.json').read_text())
    require((manifest['url'], manifest['size'], manifest['sha256']) == (URL, SIZE, DIGEST), 'manifest-trust-mismatch')
    expected = json.loads((scripts/'cli_auth_expected_tests.json').read_text())
    all_names = [n for group in expected.values() for n in group['names']]
    ordinary_names = [n for group in expected.values() if not group.get('ignored') for n in group['names']]
    require(len(all_names) == len(set(all_names)) == 62 and len(ordinary_names) == 61, 'expected-inventory')
    require(expected['actual-esm']['names'] == [ACTUAL], 'ignored-inventory')
    ordinary_log = ANSI.sub('', (evidence/'ordinary-tests.log').read_text())
    passed = re.findall(r'^test (.+) \.\.\. ok$', ordinary_log, re.M)
    require(all(passed.count(n) == 1 for n in ordinary_names), 'ordinary-selected-not-passed')
    home = pathlib.Path.home()
    cargo = pathlib.Path(shutil.which('cargo') or '')
    require(cargo.is_absolute() and cargo.is_file(), 'cargo-unavailable')
    owned_root = pathlib.Path(tempfile.mkdtemp(prefix='ens5903-private-', dir=evidence))
    root_identity = owned(owned_root, True)
    try:
        os.chmod(owned_root, 0o700)
        private_home = owned_root/'home'; private_home.mkdir(mode=0o700)
        cwd = owned_root/'cwd'; cwd.mkdir(mode=0o700)
        env = {'PATH': '/usr/bin:/bin', 'HOME': str(private_home), 'TMPDIR': str(owned_root),
               'LANG': 'C.UTF-8', 'RUST_BACKTRACE': '0', 'TOKIO_WORKER_THREADS': '2'}
        build_env = env | {'PATH': str(cargo.parent)+':/usr/bin:/bin',
                          'RUSTC': str(cargo.parent/'rustc'),
                          'CARGO_HOME': str(home/'.cargo'), 'RUSTUP_HOME': str(home/'.rustup'),
                          'RUSTC_WRAPPER': '', 'RUSTC_WORKSPACE_WRAPPER': ''}
        # Existing toolchain action is CI's compiler authority; select its reported
        # executable, never a glob or arbitrary previously installed test binary.
        output = capture([str(cargo), 'test', '--locked', '--manifest-path', str(repo/'Cargo.toml'),
                          '--bin', 'enscrive', '--no-run', '--message-format=json'], cwd, build_env)
        (evidence/'compile-tests.log').write_bytes(output)
        candidates = []
        for line in output.decode().splitlines():
            try: item = json.loads(line)
            except ValueError: continue
            if item.get('reason') == 'compiler-artifact' and item.get('executable') and item['target']['name'] == 'enscrive':
                candidates.append(pathlib.Path(item['executable']))
        require(len(candidates) == 1, 'compiler-binary-cardinality')
        test = owned_root/'tests.bin'
        shutil.copyfile(candidates[0], test); test.chmod(0o500)
        binary_digest = digest(test)
        for label, selection in expected.items():
            args = [str(test), selection['filter'], '--list']
            if selection.get('ignored'): args.append('--ignored')
            if selection.get('skip'): args += ['--skip', selection['skip']]
            data = capture(args, cwd, env, seconds=30)
            names = sorted(re.findall(r'^(.+): test$', data.decode(), re.M))
            require(names == selection['names'], 'compiled-inventory-mismatch')
            (evidence/(label+'-list.log')).write_bytes(data)
        esm = owned_root/'esm'
        accepted_identity = fetch(esm)
        activate(esm, accepted_identity)
        fixture_env = env | {'ENS5903_TEST_ESM': str(esm)}
        data = checked_call(esm, accepted_identity,
                            lambda _: capture([str(test), ACTUAL, '--ignored', '--exact', '--test-threads=1', '--nocapture'], cwd, fixture_env))
        text = ANSI.sub('', data.decode())
        require(re.findall(r'^test (.+) \.\.\. ok$', text, re.M) == [ACTUAL], 'ignored-name-not-passed')
        require(re.search(r'test result: ok\. 1 passed; 0 failed; 0 ignored;', text) is not None, 'ignored-summary')
        require(digest(test) == binary_digest, 'test-binary-changed')
        (evidence/'actual-esm.log').write_bytes(data)
        write_json(evidence/'fixture-result.json', {'passed': True, 'ordinary_selected': ordinary_names,
                   'ignored_selected': [ACTUAL], 'total': 62, 'esm': manifest,
                   'test_binary_sha256': binary_digest,
                   'helper_sha256': digest(scripts/'cli_auth_ci.py'),
                   'selection_manifest_sha256': digest(scripts/'cli_auth_expected_tests.json'),
                   'claim': 'same bytes previously signature verified; no cosign rerun; no stack/provider'})
    finally:
        current = owned(owned_root, True)
        require((current.st_dev,current.st_ino) == (root_identity.st_dev,root_identity.st_ino), 'cleanup-directory-changed')
        shutil.rmtree(owned_root)
        require(not owned_root.exists(), 'cleanup-directory-remains')

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('mode', choices=['ordinary','fixture','download-worker'])
    parser.add_argument('--repo', type=pathlib.Path)
    parser.add_argument('--evidence', type=pathlib.Path)
    parser.add_argument('--destination', type=pathlib.Path)
    args = parser.parse_args()
    if args.mode == 'download-worker':
        require(args.destination is not None and args.repo is None and args.evidence is None, 'worker-arguments')
        try:
            _fetch(args.destination, url=URL, size=SIZE, expected=DIGEST, opener=open_response)
        except Exception as error:
            raise SystemExit(str(error) if isinstance(error, Refusal) else 'download-unavailable') from None
        return
    require(args.repo is not None and args.evidence is not None and args.destination is None, 'fixture-arguments')
    repo = args.repo.resolve()
    evidence = args.evidence
    owned(evidence, True)
    require(stat.S_IMODE(evidence.stat().st_mode) == 0o700, 'evidence-directory-mode')
    try:
        (ordinary if args.mode == 'ordinary' else fixture)(repo, evidence)
    except Exception as error:
        kind = str(error) if isinstance(error, Refusal) else 'fixture-unavailable'
        if hasattr(error, 'output'):
            (evidence/(args.mode+'-failure.log')).write_bytes(error.output)
        write_json(evidence/(args.mode+'-failure.json'), failure_record(error))
        raise SystemExit(kind) from None

if __name__ == '__main__':
    main()
