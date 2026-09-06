// One native-owned snapshot in bounded chunks. A reference is consumed once;
// incomplete or failed transfers never become callback context.
import { createHash } from "node:crypto";
import { bounded } from "./runtime.mjs";

export const MAX_SNAPSHOT_BYTES = 8 * 1024 * 1024;
export const MAX_SNAPSHOT_NODES = 250000;
export class SnapshotTransfer {
	#pending;
	begin({ id, bytes, sha256 }) {
		this.#pending = undefined;
		if (typeof id !== "string" || !/^[a-zA-Z0-9-]{1,128}$/.test(id) ||
			!Number.isSafeInteger(bytes) || bytes < 2 || bytes > MAX_SNAPSHOT_BYTES ||
			typeof sha256 !== "string" || !/^[a-f0-9]{64}$/.test(sha256))
			throw new Error("Invalid native snapshot transfer");
		this.#pending = { id, sha256, bytes: Buffer.alloc(bytes), offset: 0 };
		return { id, offset: 0 };
	}
	append({ id, offset, data }) {
		try {
			const p = this.#pending;
			if (!p || id !== p.id || offset !== p.offset || typeof data !== "string" || data.length > 87384)
				throw new Error("Invalid native snapshot chunk");
			const bytes = Buffer.from(data, "base64");
			if (bytes.length === 0 || bytes.length > 65536 || bytes.toString("base64") !== data ||
				p.offset + bytes.length > p.bytes.length)
				throw new Error("Invalid native snapshot chunk");
			bytes.copy(p.bytes, p.offset);
			p.offset += bytes.length;
			return { id, offset: p.offset };
		} catch (error) {
			this.#pending = undefined;
			throw error;
		}
	}
	take(id) {
		const p = this.#pending;
		this.#pending = undefined;
		if (!p || id !== p.id || p.offset !== p.bytes.length ||
			createHash("sha256").update(p.bytes).digest("hex") !== p.sha256)
			throw new Error("Incomplete native snapshot transfer");
		const text = new TextDecoder("utf-8", { fatal: true }).decode(p.bytes);
		const snapshot = JSON.parse(text);
		if (!snapshot || typeof snapshot !== "object" || Array.isArray(snapshot))
			throw new Error("Invalid native snapshot");
		return bounded(snapshot, MAX_SNAPSHOT_BYTES, MAX_SNAPSHOT_NODES);
	}
}
