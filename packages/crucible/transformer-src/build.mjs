import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const source = path.dirname(fileURLToPath(import.meta.url));
const output = path.join(source, "..", "transformer");
fs.mkdirSync(output, { recursive: true });
fs.copyFileSync(path.join(source, "index.cjs"), path.join(output, "index.js"));
fs.copyFileSync(path.join(source, "type-id.cjs"), path.join(output, "type-id.cjs"));

fs.copyFileSync(path.join(source, "diagnostics.cjs"), path.join(output, "diagnostics.cjs"));
