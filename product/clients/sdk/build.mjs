// Uses the already pinned desktop TypeScript compiler; no package installation.
import ts from "../../apps/desktop/node_modules/typescript/lib/typescript.js";
import { mkdtempSync, readFileSync, writeFileSync, rmSync, mkdirSync, copyFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
const root = dirname(fileURLToPath(import.meta.url)), temporary = mkdtempSync(join(tmpdir(), "ag1-sdk-build-"));
try {
  writeFileSync(join(temporary, "package.json"), '{"type":"module"}');
  writeFileSync(join(temporary, "index.ts"), readFileSync(join(root, "index.ts"), "utf8").replace('"../../apps/desktop/src/generated/protocol"', '"./protocol.js"'));
  copyFileSync(join(root, "hooks.ts"), join(temporary, "hooks.ts"));
  copyFileSync(join(root, "../../apps/desktop/src/generated/protocol.ts"), join(temporary, "protocol.ts"));
  const program = ts.createProgram([join(temporary, "index.ts"), join(temporary, "protocol.ts"), join(temporary, "hooks.ts")], {
    target: ts.ScriptTarget.ES2023, module: ts.ModuleKind.NodeNext, moduleResolution: ts.ModuleResolutionKind.NodeNext,
    strict: true, declaration: true, noEmitOnError: true, types: [], outDir: join(root, "dist"), rootDir: temporary,
  });
  const result = program.emit(), diagnostics = ts.getPreEmitDiagnostics(program).concat(result.diagnostics);
  if (diagnostics.length) throw new Error(ts.formatDiagnosticsWithColorAndContext(diagnostics, { getCanonicalFileName: f => f, getCurrentDirectory: () => root, getNewLine: () => "\n" }));
  mkdirSync(join(root, "dist"), { recursive: true });
  copyFileSync(join(root, "../../LICENSE"), join(root, "LICENSE"));
  console.log("Built standalone SDK JavaScript and generated protocol declarations");
} finally { rmSync(temporary, { recursive: true, force: true }); }
