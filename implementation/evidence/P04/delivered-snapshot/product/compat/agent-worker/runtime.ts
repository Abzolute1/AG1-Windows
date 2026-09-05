import fs from "node:fs";
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
      const expected = buildArgs({
        ...p,
        modelPath: entry.path,
        alias: plan.model.id,
        host: plan.env.AG1_LLAMA_CPP_HOST ?? "127.0.0.1",
        port: Number(plan.env.AG1_LLAMA_CPP_PORT ?? 8080),
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
