// Reference execution guard: no real network or model/backend administration.
// This is defense in depth for trusted test fixtures, not an OS sandbox.
import net from 'node:net';
import child from 'node:child_process';
import { syncBuiltinESMExports } from 'node:module';
import { promisify } from 'node:util';

const ownedPorts = new Set();
const listen = net.Server.prototype.listen;
net.Server.prototype.listen = function (...args) {
  this.once('listening', () => {
    const address = this.address();
    if (address && typeof address === 'object') {
      ownedPorts.add(address.port);
      this.once('close', () => ownedPorts.delete(address.port));
    }
  });
  return listen.apply(this, args);
};
const socketConnect = net.Socket.prototype.connect;
net.Socket.prototype.connect = function (...args) {
  const first = Array.isArray(args[0]) ? args[0][0] : args[0];
  const port = Number(typeof first === 'object' ? first.port : first);
  const host = typeof first === 'object' ? first.host : args[1];
  if (!ownedPorts.has(port) || !['127.0.0.1', '::1', 'localhost'].includes(host ?? 'localhost')) {
    throw new Error('AG1_OFFLINE: only this process\'s fixture servers may be contacted');
  }
  return socketConnect.apply(this, args);
};
const fetchOriginal = globalThis.fetch;
globalThis.fetch = async (input, ...args) => {
  const url = new URL(typeof input === 'string' || input instanceof URL ? input : input.url);
  if (!ownedPorts.has(Number(url.port)) || !['127.0.0.1', '[::1]', 'localhost'].includes(url.hostname)) {
    throw new Error('AG1_OFFLINE: real fetch is forbidden');
  }
  return fetchOriginal(input, ...args);
};
for (const method of ['spawn', 'spawnSync', 'exec', 'execSync', 'execFile', 'execFileSync']) {
  const original = child[method];
  const check = (args) => {
    const command = String(args[0]);
    if (/llama[-_]?(server|cli)|llama\.cpp|(?:^|[\s/])(curl|wget|ssh)(?:\s|$)/i.test(command)) {
      throw new Error(`AG1_OFFLINE: model/network executable forbidden (${method})`);
    }
    const words = [command, ...(Array.isArray(args[1]) ? args[1] : [])].join(' ');
    if (/\bgit\b.*\b(clone|fetch|pull|ls-remote)\b/.test(words) && /https?:\/\/|ssh:\/\/|git@|git:\/\//.test(words)) {
      throw new Error('AG1_OFFLINE: remote Git transport forbidden');
    }
  };
  const wrapper = function (...args) {
    check(args);
    return original.apply(this, args);
  };
  // Preserve execFile/exec promisify's {stdout, stderr} result contract.
  if (original[promisify.custom]) {
    wrapper[promisify.custom] = function (...args) {
      check(args);
      return original[promisify.custom].apply(this, args);
    };
  }
  child[method] = wrapper;
}
syncBuiltinESMExports();
