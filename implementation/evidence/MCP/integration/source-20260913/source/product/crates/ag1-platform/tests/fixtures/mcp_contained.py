"""Owned connection-time containment probes, only within test-created paths."""
import json, os, pathlib, socket, sys
outside = pathlib.Path(sys.argv[1])
def blocked(operation):
    try:
        operation()
        return False
    except OSError:
        return True
checks = {
    'read_allowed': pathlib.Path('allowed').read_text() == 'owned',
    'read_outside_blocked': blocked(lambda: outside.read_text()),
    'symlink_escape_blocked': blocked(lambda: pathlib.Path('escape').read_text()),
    'write_allowed_blocked': blocked(lambda: pathlib.Path('allowed').write_text('changed')),
    'write_outside_blocked': blocked(lambda: outside.write_text('changed')),
    'tcp_blocked': blocked(lambda: socket.socket(socket.AF_INET, socket.SOCK_STREAM)),
    'unix_blocked': blocked(lambda: socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)),
    'private_environment_received': os.environ.get('OWNED_PRIVATE') == 'owned-containment-value',
}
for line in sys.stdin:
    r = json.loads(line)
    method = r.get('method')
    if method == 'server/discover':
        result = {'resultType':'complete','supportedVersions':['2026-07-28'],'capabilities':{'tools':{}},'ttlMs':0,'cacheScope':'private'}
    elif method == 'tools/list':
        result = {'tools':[{'name':'containment_report','description':json.dumps(checks),'inputSchema':{'type':'object','properties':{},'additionalProperties':False}}]}
    elif 'id' not in r:
        continue
    else:
        raise AssertionError('connection must not execute operations')
    print(json.dumps({'jsonrpc':'2.0','id':r['id'],'result':result}), flush=True)
