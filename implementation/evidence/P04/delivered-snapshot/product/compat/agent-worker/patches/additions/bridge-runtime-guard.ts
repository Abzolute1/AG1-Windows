// P04 D01. A persisted PID or healthy HTTP response is never ownership.
import type { ChildProcess } from 'node:child_process';
export interface BridgeRuntimeGuard {
 checkOptions(options: unknown): void;
 checkPlan(plan: { binary: { command: string; libPath?: string }; args: string[]; model: { id: string; path: string }; profile: unknown; env: NodeJS.ProcessEnv }): Promise<void>;
 owns(pid: number): boolean;
 adopt(child: ChildProcess): void;
}
let installed: BridgeRuntimeGuard | undefined;
export function installBridgeRuntimeGuard(guard: BridgeRuntimeGuard) {
 if (installed) throw new Error('Runtime guard already installed'); installed = guard;
}
export function bridgeRuntimeGuard(): BridgeRuntimeGuard {
 if (!installed) throw new Error('Guarded bridge runtime has no installed owner; model action refused'); return installed;
}
