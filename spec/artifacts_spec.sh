#shellcheck shell=bash

Describe 'artifact export and publication'
  BeforeEach 'setup_workspace'
  AfterEach 'cleanup_workspace'

  apple_toolchain_unavailable() {
    [ "$(uname -s)" != "Darwin" ] || ! command -v xcrun >/dev/null 2>&1
  }

  inspect_apple_export() {
    python3 - "$ONCE_BIN" "$WORKSPACE" "$REPO_ROOT" <<'PY'
import hashlib, json, pathlib, platform, plistlib, shutil, subprocess, sys
binary, workspace, repo = sys.argv[1:]
root = pathlib.Path(workspace)
shutil.copytree(pathlib.Path(repo) / 'crates/once-frontend/prelude/examples/apple-application-macos', root, dirs_exist_ok=True)
def run(*args):
    return json.loads(subprocess.check_output([binary, '-C', workspace, '--format', 'json', *args]))
run('build', 'Hello')
assert not list((root / '.once/out').rglob('*.zip'))
first = run('artifact', 'export', 'Hello')
directory = pathlib.Path(first['directory'])
def content(descriptor):
    path = directory / 'blobs/sha256' / descriptor['digest'].split(':')[1]
    data = path.read_bytes()
    assert len(data) == descriptor['size_bytes']
    assert 'sha256:' + hashlib.sha256(data).hexdigest() == descriptor['digest']
    return path, data
_, index = content(first['root'])
member = json.loads(index)['members']['product']
assert member['selectors'] == {'platform': 'macos', 'sdk_variant': 'device', 'architecture': platform.machine()}
_, manifest = content(member['artifact'])
manifest = json.loads(manifest)
archive, _ = content(manifest['payloads']['bundle'])
_, metadata = content(manifest['payloads']['metadata'])
metadata = json.loads(metadata)
destination = root / 'unpacked'
subprocess.run(['/usr/bin/ditto', '-x', '-k', str(archive), str(destination)], check=True)
app = destination / 'Hello.app'
subprocess.run(['/usr/bin/codesign', '--verify', '--deep', '--strict', str(app)], check=True)
plist = plistlib.loads((app / 'Contents/Info.plist').read_bytes())
assert metadata['bundle_identifier'] == plist['CFBundleIdentifier'] == 'dev.once.Hello'
assert metadata['supported_platforms'] == ['macos']
executable = app / 'Contents/MacOS' / plist['CFBundleExecutable']
uuid = subprocess.check_output(['xcrun', 'dwarfdump', '--uuid', str(executable)], text=True).split()[1]
assert metadata['binary_id'] == uuid
assert executable.stat().st_mode & 0o111
assert run('artifact', 'export', 'Hello')['root'] == first['root']
assert not (root / '.once/publications').exists()
print('signed Apple bundle, native metadata, cache replay and explicit publication verified')
PY
  }

  inspect_local_publication() {
    python3 - "$ONCE_BIN" "$WORKSPACE" "$REPO_ROOT" "$XDG_CACHE_HOME" <<'PY'
import hashlib, json, pathlib, shutil, subprocess, sys, zipfile
binary, workspace, repo, cache = sys.argv[1:]
root = pathlib.Path(workspace)
shutil.copytree(pathlib.Path(repo) / 'crates/once-frontend/prelude/examples/artifact-static-site', root, dirs_exist_ok=True)
def run(*args):
    return json.loads(subprocess.check_output([binary, '-C', workspace, '--format', 'json', *args]))
contract = run('query', 'artifacts', 'site')
assert contract['supported'] is True
first = run('artifact', 'export', 'site')
shutil.rmtree(root / '.once/out')
second = run('artifact', 'export', 'site')
assert first['root'] == second['root']
one = run('artifact', 'publish', 'site', '--provider', 'local', '--track', 'one')
two = run('artifact', 'publish', 'site', '--provider', 'local', '--track', 'two')
assert one['artifact'] == two['artifact'] == first['root']
assert one['track'] == 'one' and two['track'] == 'two'
assert len(list((root / '.once/publications').glob('*.json'))) == 2
directory = pathlib.Path(first['directory'])
def content(descriptor):
    data = (directory / 'blobs/sha256' / descriptor['digest'].split(':')[1]).read_bytes()
    assert len(data) == descriptor['size_bytes']
    assert 'sha256:' + hashlib.sha256(data).hexdigest() == descriptor['digest']
    return data
index = json.loads(content(first['root']))
manifest = json.loads(content(index['members']['product']['artifact']))
assert manifest['interfaces']['dev.once.static-http.v1']['spec']['root'] == 'public'
content(manifest['payloads']['content'])
archive = directory / 'blobs/sha256' / manifest['payloads']['content']['digest'].split(':')[1]
with zipfile.ZipFile(archive) as zipped:
    assert zipped.namelist() == ['public/index.html']
    assert b'A shareable build' in zipped.read('public/index.html')
shutil.rmtree(cache)
shutil.rmtree(root / '.once/out')
assert archive.is_file()
requests = [
    {'jsonrpc': '2.0', 'id': 1, 'method': 'tools/list', 'params': {}},
    {'jsonrpc': '2.0', 'id': 2, 'method': 'tools/call', 'params': {'name': 'once_publish_artifacts', 'arguments': {'target': 'site', 'provider': 'local'}}},
]
process = subprocess.run([binary, '-C', workspace, 'mcp'], input=''.join(json.dumps(r) + '\n' for r in requests), text=True, capture_output=True, check=True)
responses = [json.loads(line) for line in process.stdout.splitlines()]
names = [tool['name'] for tool in responses[0]['result']['tools']]
assert 'once_query_artifacts' in names and 'once_publish_artifacts' not in names
assert responses[1]['result']['isError'] is True
print('portable content, cache replay, independent retention and explicit publication verified')
PY
  }

  inspect_native_names_and_links() {
    python3 - "$ONCE_BIN" "$WORKSPACE" <<'PY'
import json, pathlib, stat, subprocess, sys, zipfile
binary, workspace = sys.argv[1:]
root = pathlib.Path(workspace)
(root / 'data').mkdir()
(root / 'data/a').write_text('alpha')
(root / 'data/b').write_text('beta')
(root / 'alias').symlink_to('data/a')
(root / 'linked').symlink_to('data', target_is_directory=True)
def export(name):
    (root / 'once.toml').write_text('[[target]]\nname = ' + json.dumps(name, ensure_ascii=False) + '\nkind = "artifact"\nsrcs = ["data/**/*", "alias", "linked"]\n')
    result = json.loads(subprocess.check_output([binary, '-C', workspace, '--format', 'json', 'artifact', 'export', name]))
    directory = pathlib.Path(result['directory']) / 'blobs/sha256'
    def content(descriptor):
        return directory / descriptor['digest'].split(':')[1]
    index = json.loads(content(result['root']).read_bytes())
    manifest = json.loads(content(index['members']['product']['artifact']).read_bytes())
    return result['root'], content(manifest['payloads']['content'])
first = None
for name in ('My App', '冰块', 'n' * 140):
    identity, archive = export(name)
    if first is None: first = identity
    assert identity == first
    with zipfile.ZipFile(archive) as zipped:
        assert zipped.namelist() == ['alias', 'data/a', 'data/b', 'linked']
        for path, target in [('alias', b'data/a'), ('linked', b'data')]:
            assert stat.S_ISLNK(zipped.getinfo(path).external_attr >> 16)
            assert zipped.read(path) == target
        assert zipped.read('data/a') == b'alpha'
(root / 'alias').unlink()
(root / 'alias').symlink_to('data/b')
changed, archive = export('My App')
assert changed != first
with zipfile.ZipFile(archive) as zipped:
    assert zipped.read('alias') == b'data/b'
assert export('My App')[0] == changed
print('native names, file and directory links, retargeting and cache replay verified')
PY
  }

  inspect_tuist_transport() {
    python3 - "$ONCE_BIN" "$WORKSPACE" <<'PY'
import hashlib, http.server, json, os, pathlib, subprocess, sys, threading
binary, workspace = sys.argv[1:]
root = pathlib.Path(workspace)
(root / 'modules').mkdir()
(root / 'modules/product.star').write_text('''
def _impl(ctx):
    out = declare_output("upload.bin")
    copy_path("upload.bin", out)
    metadata = declare_output("metadata.json")
    write_path(metadata, '{"type":"app_bundle","binary_id":"fixture","build_version":"1"}')
    return {"artifact_info": _artifact_info([{"name": "product", "payloads": {"bundle": {"path": out, "media_type": "application/zip"}, "metadata": {"path": metadata, "media_type": "application/json"}}, "interfaces": {"dev.tuist.preview.v1": {"payload": "bundle", "spec": {"metadata_payload": "metadata"}}}}])}
product = target_kind(impl = _impl, providers = ["once_artifact_info"], capabilities = [capability("artifact", ["default"])])
''')
payload = b'a' * (8 * 1024 * 1024) + b'final-part-123'
(root / 'upload.bin').write_bytes(payload)
state = {'mode': 'ok', 'parts': [], 'errors': [], 'calls': []}
class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, *args): pass
    def reply(self, status, value):
        data = json.dumps(value).encode()
        self.send_response(status)
        self.send_header('Content-Length', str(len(data)))
        self.end_headers()
        self.wfile.write(data)
    def do_POST(self):
        try:
            assert self.path.startswith('/api/projects/acme/app/previews/'), self.path
            assert self.headers['Authorization'] == 'Bearer fixture-token'
            body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
            operation = self.path.rsplit('/', 1)[1]
            state['calls'].append(operation)
            if operation == 'start':
                assert body['binary_id'] == 'fixture' and body['track'] == 'review'
                if state['mode'] in ('duplicate', 'unauthorized'):
                    return self.reply(409 if state['mode'] == 'duplicate' else 401, {})
                return self.reply(200, {'data': {'upload_id': 'upload', 'app_build_id': 'build'}})
            assert body['preview_id'] == body['app_build_id'] == 'build'
            if operation == 'generate-url':
                part = body['multipart_upload_part']
                assert part['upload_id'] == 'upload'
                return self.reply(200, {'data': {'url': f'http://127.0.0.1:{self.server.server_port}/part/{part["part_number"]}'}})
            assert operation == 'complete'
            assert body['multipart_upload_parts'] == {'upload_id': 'upload', 'parts': [{'part_number': 1, 'etag': '"one"'}, {'part_number': 2, 'etag': '"two"'}]}
            assert b''.join(state['parts']) == payload
            return self.reply(500 if state['mode'] == 'complete-failed' else 200, {'url': f'http://127.0.0.1:{self.server.server_port}/preview'})
        except Exception as error:
            state['errors'].append(str(error))
            self.reply(400, {})
    def do_PUT(self):
        assert self.headers.get('Authorization') is None
        state['parts'].append(self.rfile.read(int(self.headers['Content-Length'])))
        self.send_response(500 if state['mode'] == 'part-failed' else 200)
        self.send_header('ETag', '"one"' if self.path.endswith('/1') else '"two"')
        self.send_header('Content-Length', '0')
        self.end_headers()
server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
thread = threading.Thread(target=server.serve_forever, daemon=True)
thread.start()
(root / 'once.toml').write_text(f'''[modules]
paths = ["modules/*.star"]
[infrastructures.fixture]
kind = "tuist"
url = "http://127.0.0.1:{server.server_port}"
account = "acme"
project = "app"
[infrastructure.artifacts]
name = "fixture"
[[target]]
name = "product"
kind = "product"
srcs = ["upload.bin"]
''')
try:
    for mode in ('ok', 'duplicate', 'unauthorized', 'part-failed', 'complete-failed'):
        state.update(mode=mode, parts=[], calls=[])
        process = subprocess.run([binary, '-C', workspace, '--format', 'json', 'artifact', 'publish', 'product', '--track', 'review'], env={**os.environ, 'TUIST_TOKEN': 'fixture-token'}, text=True, capture_output=True)
        assert not state['errors'], state['errors']
        if mode == 'ok':
            assert process.returncode == 0, process.stderr
            receipt = json.loads(process.stdout)
            assert receipt['url'].endswith('/preview')
        else:
            assert process.returncode != 0, (mode, process.stdout)
            assert state['calls'].count('start') == 1, state['calls']
            if mode == 'duplicate': assert 'duplicate preview' in process.stderr
            if mode == 'unauthorized': assert '401' in process.stderr
            if mode == 'part-failed': assert 'complete' not in state['calls']
        assert len(list((root / '.once/publications').glob('*.json'))) == 1
finally:
    server.shutdown()
    server.server_close()
    thread.join()
print('authenticated multipart bytes, legacy ids, completion and failure semantics verified')
PY
  }

  It 'retains verified content across cache replay and publishes only explicitly'
    When call inspect_local_publication
    The status should be success
    The stdout should include 'explicit publication verified'
  End

  It 'exports native target names and preserves selected links across retargeting'
    Skip if 'requires symbolic links' test "$(uname -s)" = "Windows_NT"
    When call inspect_native_names_and_links
    The status should be success
    The stdout should include 'retargeting and cache replay verified'
  End

  It 'publishes through the Tuist multipart contract without retrying unknown outcomes'
    When call inspect_tuist_transport
    The status should be success
    The stdout should include 'failure semantics verified'
  End

  It 'exports a signed Apple application with matching native preview metadata'
    Skip if 'requires the Apple toolchain' apple_toolchain_unavailable
    When call inspect_apple_export
    The status should be success
    The stdout should include 'explicit publication verified'
  End
End
