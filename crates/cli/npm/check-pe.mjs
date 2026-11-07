import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const pe = readFileSync(process.argv[2]);
const header = pe.readUInt32LE(0x3c);
assert.equal(pe.toString("ascii", header, header + 4), "PE\0\0");
const count = pe.readUInt16LE(header + 6);
const optional = header + 24;
assert.equal(pe.readUInt16LE(optional), 0x20b, "Expected PE32+");
const sections = optional + pe.readUInt16LE(header + 20);
function offset(rva) {
	for (let i = 0; i < count; i++) {
		const section = sections + i * 40;
		const start = pe.readUInt32LE(section + 12);
		if (rva >= start && rva < start + Math.max(pe.readUInt32LE(section + 8), pe.readUInt32LE(section + 16)))
			return rva - start + pe.readUInt32LE(section + 20);
	}
	throw new Error(`Unmapped RVA ${rva}`);
}
const imports = [];
for (let entry = offset(pe.readUInt32LE(optional + 120)); pe.readUInt32LE(entry + 12); entry += 20) {
	const name = offset(pe.readUInt32LE(entry + 12));
	imports.push(pe.toString("ascii", name, pe.indexOf(0, name)));
}
assert.ok(imports.length > 0, "Expected Windows imports");
console.log(`PE imports: ${imports.join(", ")}`);
assert.ok(
	imports.every((name) => !/^(vcruntime|msvcp|msvcr)/i.test(name)),
	"Windows CRT must be statically linked",
);
console.log("No VCRUNTIME140 imports");
