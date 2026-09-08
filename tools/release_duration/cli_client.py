"""Durable JSONL client for P14; no model loop or replacement scheduler."""
import copy
import json
import os
from pathlib import Path
import queue
import subprocess
import threading
import time
import uuid


class NativeClient:
    def __init__(self, binary, workspace, state, config, output, heap_library=None, heap_interval=60):
        self.output = Path(output)
        self.output.mkdir(mode=0o700)
        self.frames = queue.Queue(maxsize=4096)
        self.view = None
        self.failure = None
        self.replies = {}
        self.closed = False
        self.output_ended = False
        self.log = (self.output / 'protocol.jsonl').open('x')
        self.lock = threading.Lock()
        self.stderr = (self.output / 'stderr.log').open('x')
        env = dict(os.environ)
        if heap_library:
            env.update(LD_PRELOAD=str(heap_library),
                       AG1_HEAP_SAMPLE_PREFIX=str(self.output.absolute() / 'heap'),
                       AG1_HEAP_SAMPLE_SECONDS=str(heap_interval))
        self.argv = [str(binary), '--workspace', str(workspace), '--state', str(state),
                     '--config', str(config), '--mode', 'jsonl']
        self.process = subprocess.Popen(self.argv, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                        stderr=self.stderr, env=env)
        self.pidfd = os.pidfd_open(self.process.pid)
        self.reader = threading.Thread(target=self._read, daemon=True)
        self.reader.start()

    def _record(self, direction, data):
        with self.lock:
            self.log.write(json.dumps({'monotonic_ns': time.monotonic_ns(),
                                       'direction': direction, 'data': data}) + '\n')
            self.log.flush()
            os.fsync(self.log.fileno())

    def _read(self):
        try:
            while True:
                line = self.process.stdout.readline(65538)
                if not line:
                    break
                if len(line) > 65537 or not line.endswith(b'\n'):
                    raise RuntimeError('Unbounded or truncated CLI protocol frame')
                frame = json.loads(line)
                self._record('stdout', frame)
                self.frames.put(frame, timeout=5)
        except Exception as error:
            self.failure = repr(error)
        finally:
            self.frames.put(None, timeout=5)

    def pump(self, timeout=1):
        try:
            frame = self.frames.get(timeout=timeout)
        except queue.Empty:
            if self.failure:
                raise RuntimeError(self.failure)
            if self.process.poll() is not None:
                raise RuntimeError(f'CLI exited {self.process.returncode}')
            return
        if frame is None:
            self.output_ended = True
            raise RuntimeError(f'CLI output ended: {self.failure or self.process.poll()}')
        if frame['type'] == 'client':
            self.view = frame['data']
        elif frame['type'] == 'bridge' and frame['data']['kind'] == 'reply':
            self.replies[frame['data']['id']] = frame['data']
        elif frame['type'] == 'error':
            raise RuntimeError(f'CLI protocol error: {frame}')

    def ready(self, seconds=1800):
        deadline = time.monotonic() + seconds
        while self.view is None:
            if time.monotonic() >= deadline:
                raise TimeoutError('CLI admission timed out')
            self.pump()
        return self.view

    def send(self, command, scope=None, revision=None):
        self.ready()
        snapshot = self.view['snapshot']
        identity = 'p14-' + uuid.uuid4().hex
        envelope = {'version': snapshot['version'], 'id': identity,
                    'scope': copy.deepcopy(scope or snapshot['scope']),
                    'expected_revision': revision or snapshot['revision'], 'command': command}
        frame = {'type': 'command', 'data': envelope}
        self._record('stdin', frame)
        encoded = json.dumps(frame).encode() + b'\n'
        if len(encoded) > 65536:
            raise ValueError('Command requires artifact transport')
        self.process.stdin.write(encoded)
        self.process.stdin.flush()
        return envelope

    def reply(self, envelope, seconds=60):
        deadline = time.monotonic() + seconds
        while envelope['id'] not in self.replies:
            if time.monotonic() >= deadline:
                raise TimeoutError('CLI acknowledgement timed out')
            self.pump()
        return self.replies.pop(envelope['id'])

    def execute(self, command, seconds=1200):
        started = time.monotonic_ns()
        envelope = self.send(command)
        acknowledgement = self.reply(envelope)
        if acknowledgement['status'] == 'query':
            return {'envelope': envelope, 'acknowledgement': acknowledgement,
                    'started_monotonic_ns': started, 'finished_monotonic_ns': time.monotonic_ns()}
        if acknowledgement['status'] != 'accepted':
            raise RuntimeError(f'Command rejected: {acknowledgement}')
        deadline = time.monotonic() + seconds
        result = acknowledgement['data']
        operation = result['operation'] if result['kind'] == 'accepted' else result['operation']['operation']
        while result['kind'] != 'settled':
            if time.monotonic() >= deadline:
                # EOF drains the CLI; it does not cancel an active generation.
                # Cancel this exact accepted operation before closing its input.
                cleanup = {'command': envelope['id'], 'operation': operation,
                           'started_monotonic_ns': time.monotonic_ns()}
                try:
                    self._cancel_expired(envelope, operation, cleanup)
                except Exception as error:
                    cleanup['error'] = repr(error)
                cleanup['finished_monotonic_ns'] = time.monotonic_ns()
                with (self.output / ('timeout-' + envelope['id'] + '.json')).open('x') as stream:
                    json.dump(cleanup, stream, indent=2)
                    stream.write('\n')
                    stream.flush()
                    os.fsync(stream.fileno())
                raise TimeoutError(f'CLI command {envelope["id"]} did not settle')
            self.pump(timeout=.2)
            query = self.send({'kind': 'command_result', 'command': envelope['id']},
                              scope=envelope['scope'])
            response = self.reply(query)
            result = response['data']
            if response['status'] == 'rejected' or result['kind'] == 'rejected':
                raise RuntimeError(f'Command receipt lookup failed: {response}')
        if result['operation'] != {'operation': operation, 'status': 'completed'}:
            raise RuntimeError(f'Command failed: {result}')
        while self.view['details']['activity'].get('last_terminal') != result['operation']:
            if time.monotonic() >= deadline:
                raise TimeoutError('Completed receipt was not projected into the connected client')
            self.pump(timeout=.2)
        return {'envelope': envelope, 'acknowledgement': acknowledgement, 'terminal': result,
                'started_monotonic_ns': started, 'finished_monotonic_ns': time.monotonic_ns()}

    def _cancel_expired(self, original, operation, cleanup, seconds=60):
        cancellation = self.send({'kind': 'cancel', 'operation': operation})
        cleanup['cancellation'] = cancellation
        cleanup['acknowledgement'] = self.reply(cancellation, seconds=seconds)
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            query = self.send({'kind': 'command_result', 'command': original['id']},
                              scope=original['scope'])
            response = self.reply(query, seconds=max(.001, deadline-time.monotonic()))
            result = response.get('data', {})
            if result.get('kind') == 'settled':
                if result['operation']['operation'] != operation:
                    raise RuntimeError('Timeout cleanup returned a different operation')
                cleanup['terminal'] = result
                cleanup['settled'] = True
                return
            if response['status'] == 'rejected' or result.get('kind') == 'rejected':
                if result.get('error', {}).get('family') == 'native_command_recovery':
                    # Cancellation delivery is not proof that remote inference
                    # or native effects settled. Preserve the custody fence.
                    cleanup['settled'] = False
                    cleanup['recovery_required'] = True
                    cleanup['unsettled_reply'] = response
                    return
                raise RuntimeError(f'Timeout receipt lookup failed: {response}')
            self.pump(timeout=.2)
        raise TimeoutError('Expired operation did not settle after cancellation')

    def close(self, seconds=1800):
        if self.closed:
            return
        self.process.stdin.close()
        deadline = time.monotonic() + seconds
        while self.process.poll() is None:
            if time.monotonic() >= deadline:
                raise TimeoutError('CLI did not drain; retained ownership needs recovery')
            try:
                self.pump(timeout=.2)
            except RuntimeError:
                if self.output_ended:
                    self.process.wait(timeout=max(.001, deadline-time.monotonic()))
                elif self.process.poll() is None:
                    raise
        self.reader.join(timeout=5)
        if self.reader.is_alive():
            raise RuntimeError('CLI output reader did not finish')
        os.close(self.pidfd)
        self.stderr.close()
        self.log.close()
        self.closed = True
        if self.process.returncode:
            raise RuntimeError(f'CLI drain failed: {self.process.returncode}')
        if self.failure:
            raise RuntimeError('CLI output recording failed: '+self.failure)
