import fs from "node:fs";
import { join } from "node:path";
import { createHash, randomUUID } from "node:crypto";
export const digest = (value: unknown) =>
  createHash("sha256")
    .update(typeof value === "string" ? value : JSON.stringify(value))
    .digest("hex");
export function atomic(path: string, data: unknown) {
  const temp = path + "." + randomUUID();
  const fd = fs.openSync(temp, "wx", 0o600);
  try {
    fs.writeFileSync(fd, JSON.stringify(data));
    fs.fsyncSync(fd);
  } finally {
    fs.closeSync(fd);
  }
  try {
    fs.renameSync(temp, path);
    const d = fs.openSync(join(path, ".."), "r");
    try {
      fs.fsyncSync(d);
    } finally {
      fs.closeSync(d);
    }
  } finally {
    if (fs.existsSync(temp)) fs.unlinkSync(temp);
  }
}
export function syncTree(root: string) {
  for (const entry of fs.readdirSync(root, { withFileTypes: true })) {
    if (entry.name.endsWith(".lock")) continue;
    const path = join(root, entry.name);
    if (entry.isSymbolicLink()) throw new Error("Symlink in private durable state");
    if (entry.isDirectory()) syncTree(path);
    else {
      const fd = fs.openSync(path, "r");
      try {
        fs.fsyncSync(fd);
      } finally {
        fs.closeSync(fd);
      }
    }
  }
  const d = fs.openSync(root, "r");
  try {
    fs.fsyncSync(d);
  } finally {
    fs.closeSync(d);
  }
}
export class Ledger {
  readonly root: string;
  revision = 0n;
  constructor(root: string) {
    this.root = root;
    fs.mkdirSync(root, { recursive: true, mode: 0o700 });
    const seq = join(root, "revision.json");
    if (fs.existsSync(seq)) this.revision = BigInt(JSON.parse(fs.readFileSync(seq, "utf8")));
  }
  path(id: string) {
    if (!/^[a-zA-Z0-9_.:-]{1,128}$/.test(id)) throw new Error("Invalid command ID");
    return join(this.root, id + ".command.json");
  }
  get(id: string) {
    const path = this.path(id);
    return fs.existsSync(path) ? JSON.parse(fs.readFileSync(path, "utf8")) : undefined;
  }
  save(id: string, record: any) {
    atomic(this.path(id), record);
    this.revision++;
    atomic(join(this.root, "revision.json"), String(this.revision));
  }
  recover() {
    let count = 0;
    for (const name of fs.readdirSync(this.root)) {
      if (!name.endsWith(".command.json")) continue;
      const id = name.slice(0, -13),
        r = this.get(id);
      if (r.status === "intent" || r.status === "accepted") {
        this.save(id, {
          ...r,
          status: "interrupted",
          detail:
            "Worker exited before durable completion; effects may have happened. No automatic replay.",
        });
        count++;
      }
    }
    return count;
  }
}
