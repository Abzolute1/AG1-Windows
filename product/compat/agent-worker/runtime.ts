import fs from "node:fs";
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { isDeepStrictEqual } from "node:util";
export function runtimeGuard(config: any, profilePath: string, buildArgs: (args: any) => string[]) {
  const owned = new Map<number, { child: any; identity: string }>();
  function identity(pid: number) {
    const stat = fs.readFileSync(`/proc/${pid}/stat`, "utf8");
    const exe = fs.statSync(`/proc/${pid}/exe`, { bigint: true });
    return `${stat.slice(stat.lastIndexOf(")") + 2).split(" ")[19]}:${exe.dev}:${exe.ino}`;
  }
  const hash = (path: string) => {
    const h = createHash("sha256"),
      fd = fs.openSync(path, "r"),
      b = Buffer.alloc(4 * 1024 * 1024);
    try {
      let n;
      while ((n = fs.readSync(fd, b)) > 0) h.update(b.subarray(0, n));
      return h.digest("hex");
    } finally {
      fs.closeSync(fd);
    }
  };
  return {
    checkOptions(options: any) {
      if (
        options.vramTune ||
        options.forcedNCpuMoe !== undefined ||
        options.useModelProfileContext === false
      )
        throw new Error("Tuning/context overrides are unavailable in the preserved bridge");
      if (
        hash(config.profile_file) !== config.profile_sha256 ||
        hash(profilePath) !== config.profile_sha256
      )
        throw new Error("Runtime profile drift; review required");
      // Observe resources only. A PID found here never becomes a stop handle.
      for (const name of fs.readdirSync("/proc")) {
        if (!/^\d+$/.test(name)) continue;
        try {
          if (fs.readFileSync(`/proc/${name}/comm`, "utf8").trim() === "llama-server" && !this.owns(Number(name)))
            throw new Error("External model process owns the runtime window; its owner must release it");
        } catch (error: any) { if (error?.code !== "ENOENT" && error?.code !== "ESRCH") throw error; }
      }
      const gpu = spawnSync("/usr/bin/nvidia-smi", ["--query-compute-apps=pid", "--format=csv,noheader,nounits"],
        { encoding: "utf8", timeout: 5000, maxBuffer: 65536, stdio: ["ignore", "pipe", "ignore"] });
      if (gpu.error || gpu.status !== 0) throw new Error("Cannot verify GPU runtime ownership");
      for (const line of gpu.stdout.trim().split("\n").filter(Boolean)) {
        if (!/^\d+$/.test(line.trim()) || !this.owns(Number(line)))
          throw new Error("External GPU process owns the runtime window; its owner must release it");
      }
      const env = options.env ?? process.env;
      const permitted = new Set([
        "AG1_LLAMA_CPP",
        "AG1_LLAMA_CPP_PORT",
        "AG1_LLAMA_CPP_HOST",
        "AG1_LLAMA_MODEL",
      ]);
      for (const key of Object.keys(env))
        if (key.startsWith("AG1_LLAMA_") && !permitted.has(key))
          throw new Error("Unreviewed runtime override: " + key);
    },
    async checkPlan(plan: any) {
      const entry = config.models[plan.model.id];
      if (
        !entry ||
        plan.model.path !== entry.path ||
        !isDeepStrictEqual(plan.profile, entry.profile)
      )
        throw new Error("Unknown or changed model profile");
      const p = entry.profile;
      if (plan.binary.command !== p.serverPath || plan.binary.libPath !== p.libPath)
        throw new Error("Tuned binary/library selection drift");
      const tier = p.contextBatchTiers?.find((t: any) => p.contextWindow <= t.maxContextWindow);
      const rawPort = plan.env.LLAMA_CPP_PORT ?? plan.env.AG1_LLAMA_CPP_PORT;
      const port = Number(rawPort);
      const expected = buildArgs({
        ...p,
        modelPath: entry.path,
        alias: plan.model.id,
        host: plan.env.LLAMA_CPP_HOST ?? plan.env.AG1_LLAMA_CPP_HOST ?? "127.0.0.1",
        port: Number.isInteger(port) && port > 0 ? port : 18080,
        contextWindow: p.contextWindow,
        batchSize: tier?.batchSize ?? p.batchSize,
        ubatchSize: tier?.ubatchSize ?? p.ubatchSize,
      });
      if (!isDeepStrictEqual(plan.args, expected))
        throw new Error("Launch argv differs from preserved profile");
      for (const item of config.material) {
        const s = fs.statSync(item.path, { bigint: true });
        const signature = [s.dev, s.ino, s.size, s.mtimeNs, s.ctimeNs].map(String);
        if (!isDeepStrictEqual(signature, item.signature))
          throw new Error("Runtime material identity changed; revalidation required: " + item.path);
      }
    },
    owns(pid: number) {
      const entry = owned.get(pid);
      if (!entry || entry.child.exitCode !== null || entry.child.signalCode !== null) return false;
      try {
        return identity(pid) === entry.identity;
      } catch {
        return false;
      }
    },
    adopt(child: any) {
      if (!child.pid) throw new Error("Missing process identity");
      owned.set(child.pid, { child, identity: identity(child.pid) });
      child.once("exit", () => owned.delete(child.pid));
    },
  };
}
