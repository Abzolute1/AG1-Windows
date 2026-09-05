import fs from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import { fileURLToPath, pathToFileURL } from "node:url";
import { createHash } from "node:crypto";
await import("../offline_guard.mjs");
const root = path.resolve(
	path.dirname(fileURLToPath(import.meta.url)),
	"../..",
);
const base = path.join(
	root,
	"reference/ag1-executable/packages/coding-agent/dist",
);
const { createReadToolDefinition } = await import(
	pathToFileURL(path.join(base, "core/tools/read.js"))
);
const { loadPhoton } = await import(
	pathToFileURL(path.join(base, "utils/photon.js"))
);
const photon = await loadPhoton();
assert(photon);
const dir = path.join(root, "fixtures/tools/images"),
	capture = process.argv.includes("--capture");
function save(name, bytes) {
	const file = path.join(dir, name);
	if (capture) {
		assert(!fs.existsSync(file));
		fs.writeFileSync(file, bytes);
	} else assert.deepEqual(bytes, fs.readFileSync(file));
}
for (const name of ["small", "wide"]) {
	const image = photon.PhotonImage.new_from_byteslice(
		fs.readFileSync(path.join(dir, `${name}.png`)),
	);
	try {
		save(`${name}.webp`, Buffer.from(image.get_bytes_webp()));
		if (name === "wide") {
			const jpeg = Buffer.from(image.get_bytes_jpeg(80));
			const exif = Buffer.from(
				"45786966000049492a0008000000010012010300010000000600000000000000",
				"hex",
			);
			const length = Buffer.alloc(2);
			length.writeUInt16BE(exif.length + 2);
			save(
				"orientation-wide.jpg",
				Buffer.concat([
					jpeg.subarray(0, 2),
					Buffer.from([0xff, 0xe1]),
					length,
					exif,
					jpeg.subarray(2),
				]),
			);
		}
	} finally {
		image.free();
	}
}
const temporary = fs.mkdtempSync(path.join(root, ".local/p09-image-extended-"));
const rows = [];
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
try {
	for (const name of [
		"small.webp",
		"wide.webp",
		"orientation-wide.jpg",
		"padded-small.png",
	]) {
		const input =
			name === "padded-small.png"
				? Buffer.concat([
						fs.readFileSync(path.join(dir, "small.png")),
						Buffer.alloc(4 * 1024 * 1024),
					])
				: fs.readFileSync(path.join(dir, name));
		fs.writeFileSync(path.join(temporary, name), input);
		for (const resize of [true, false]) {
			const result = await createReadToolDefinition(temporary, {
				autoResizeImages: resize,
			}).execute("image", { path: name });
			const content = result.content.find((c) => c.type === "image");
			assert(content);
			const image = Buffer.from(content.data, "base64");
			const decoded = photon.PhotonImage.new_from_byteslice(image);
			const dimensions = [decoded.get_width(), decoded.get_height()];
			decoded.free();
			const unchanged = image.equals(input);
			let reference_file = null;
			if (!unchanged) {
				reference_file = `extended-${name}.png`;
				save(reference_file, image);
			}
			rows.push({
				name,
				resize,
				input_sha256: hash(input),
				input_bytes: input.length,
				text: result.content[0].text,
				mime: content.mimeType,
				image_sha256: hash(image),
				image_bytes: image.length,
				dimensions,
				unchanged,
				reference_file,
			});
		}
	}
} finally {
	fs.rmSync(temporary, { recursive: true, force: true });
}
const output = {
	version: 1,
	source_commit: "e30f0b4b238ee5618928bcd17b4571eb046e7038",
	rows,
};
const target = path.join(root, "fixtures/tools/image-extended-oracle.json");
if (capture) {
	assert(!fs.existsSync(target));
	fs.writeFileSync(target, JSON.stringify(output, null, 2) + "\n");
} else assert.deepEqual(output, JSON.parse(fs.readFileSync(target, "utf8")));
console.log(
	JSON.stringify({
		status: "passed",
		image_cases: rows.length,
		model_actions: 0,
		browser_actions: 0,
		network_actions: 0,
	}),
);
