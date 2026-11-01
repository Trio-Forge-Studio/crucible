import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const source = path.dirname(fileURLToPath(import.meta.url));
const output = path.join(source, "..", "transformer");
fs.mkdirSync(output, { recursive: true });
for (const file of fs.readdirSync(source).filter((name) => name.endsWith(".cjs")))
	fs.copyFileSync(path.join(source, file), path.join(output, file === "index.cjs" ? "index.js" : file));
