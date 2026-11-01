const vm = require("node:vm");
const harness = require("./network-harness.cjs");
const { fs, path, ts } = harness;
const fixture = path.join(__dirname, "fixtures/state-data");
function project(files, options = {}) {
	const game = harness.project(files, options);
	const pkg = path.join(game, "node_modules/@trioforge/crucible/out");
	for (const [file, target, entry, line] of [
		["state.d.ts", "state.d.ts", "index.d.ts", 'export * from "./state";'],
		["data.d.ts", "data.d.ts", "index.d.ts", 'export * from "./data";'],
		["server-data.d.ts", "server/data.d.ts", "server/index.d.ts", 'export * from "./data";'],
		["emit.d.ts", "emit/data.d.ts", "emit/index.d.ts", 'export * from "./data";'],
	]) {
		fs.copyFileSync(path.join(fixture, file), path.join(pkg, target));
		const index = path.join(pkg, entry);
		if (!fs.readFileSync(index, "utf8").includes(line)) fs.appendFileSync(index, `\n${line}\n`);
	}
	return game;
}
const N = (name) => `__crucible.n.${name}`;
const S = N("string"),
	F = N("f64"),
	B = N("bool"),
	Nil = N("nil");
const L = (v) => `${N("literal")}(${JSON.stringify(v)})`;
const U = (...v) => `${N("union")}([${v.join(", ")}])`;
const A = (v) => `${N("array")}(${v})`;
const M = (k, v) => `${N("map")}(${k}, ${v})`;
const O = (fields) =>
	`${N("object")}([${fields.map(([name, node]) => `[${JSON.stringify(name)}, ${node}]`).join(", ")}])`;
const V = (key, value, body) => `${N("variant")}(${JSON.stringify(key)}, ${JSON.stringify(value)}, ${body})`;
function calls(text, names) {
	const sf = ts.createSourceFile("emitted.ts", text, ts.ScriptTarget.Latest, true);
	const found = new Map();
	const walk = (node) => {
		if (
			ts.isCallExpression(node) &&
			ts.isPropertyAccessExpression(node.expression) &&
			names.includes(node.expression.name.text) &&
			node.expression.expression.getText() === "__crucible"
		)
			found.set(node.arguments[0].text, {
				types: (node.typeArguments ?? []).map((t) => t.getText(sf).replaceAll(/\s+/g, " ")),
				args: node.arguments.map((a) => a.getText(sf).replaceAll(/\s+/g, " ")),
			});
		ts.forEachChild(node, walk);
	};
	walk(sf);
	return found;
}
const diagnosticsIn = (result, game, file) =>
	result.diagnostics
		.filter((d) => path.relative(game, d.file.fileName).replaceAll("\\", "/") === file)
		.map((d) => d.messageText);
const kind = (k) => ({ kind: k });
const builders = {
	...Object.fromEntries(
		["nil", "bool", "f64", "f32", "u8", "u16", "u32", "string", "vector3", "color3", "cframe", "dynamic"].map(
			(k) => [k, kind(k)],
		),
	),
	literal: (value) => ({ kind: "literal", value }),
	instance: (classes) => ({ kind: "instance", classes }),
	object: (fields) => ({ kind: "object", fields }),
	array: (element) => ({ kind: "array", element }),
	tuple: (elements) => ({ kind: "tuple", elements }),
	map: (key, value) => ({ kind: "map", key, value }),
	union: (branches) => ({ kind: "union", branches }),
	variant: (key, value, body) => ({ kind: "variant", key, value, body }),
};
const evaluate = (text) => vm.runInNewContext(`(${text})`, { __crucible: { n: builders } });
// The probes' own printer and node count (synthesis-v3 probes/lower/persist.cjs), so expectations stay the model's.
function describe(node, marked = new Set(), prefix = []) {
	switch (node.kind) {
		case "literal":
			return JSON.stringify(node.value);
		case "object":
			return `{ ${node.fields
				.map(
					([k, v]) =>
						`${k}${marked.has(JSON.stringify([...prefix, k])) ? " (serverOnly)" : ""}: ${describe(v, marked, [...prefix, k])}`,
				)
				.join(", ")} }`;
		case "array":
			return `${describe(node.element)}[]`;
		case "tuple":
			return `[${node.elements.map((e) => describe(e)).join(", ")}]`;
		case "map":
			return `{ [${describe(node.key)}]: ${describe(node.value)} }`;
		case "union":
			return node.branches.map((b) => describe(b)).join(" | ");
		case "variant":
			return `{ ${node.key}: ${JSON.stringify(node.value)}, ...${describe(node.body)} }`;
		case "nil":
			return "undefined";
		case "f64":
			return "number";
		case "bool":
			return "boolean";
		default:
			return node.kind;
	}
}
function countNodes(node) {
	let c = 1;
	for (const child of node.fields?.map((f) => f[1]) ?? []) c += countNodes(child);
	for (const child of [node.element, node.value, node.body].filter((x) => x && typeof x === "object"))
		c += countNodes(child);
	for (const child of [...(node.elements ?? []), ...(node.branches ?? [])]) c += countNodes(child);
	return c;
}
// Reads a probe's lower.out: one entry per declaration with its verdict and detail lines.
function verdicts(file) {
	const entries = new Map();
	let current;
	for (const line of fs.readFileSync(path.join(fixture, file), "utf8").replaceAll("\r", "").split("\n")) {
		const head = /^(ok |BAD) \S+:\d+ (\S+) /.exec(line);
		if (head) {
			current = { name: head[2], verdict: /: (lowers|rejected)$/.exec(line)?.[1], details: [] };
			entries.set(current.name, current);
		} else if (current && !current.verdict) current.verdict = /: (lowers|rejected)$/.exec(line)?.[1];
		else if (current && line.startsWith("      ")) current.details.push(line.trim());
		else current = undefined;
	}
	for (const entry of entries.values()) {
		entry.diagnostics = entry.details
			.map((d) => /^(CRU\d{3}) \S+ (\S+?): /.exec(d))
			.filter(Boolean)
			.map(([, code, at]) => ({ code, at }));
		const shape = entry.details.map((d) => /^(\d+) nodes: (.*)$/.exec(d)).find(Boolean);
		if (shape) entry.shape = { nodes: Number(shape[1]), text: shape[2] };
		const handles = entry.details
			.map((d) =>
				/^persist (\d+) nodes; serverOnly: \[(.*)\]; replica (\d+) nodes; (\d+) view declarations$/.exec(d),
			)
			.find(Boolean);
		if (handles)
			entry.handles = {
				persist: Number(handles[1]),
				serverOnly: handles[2],
				replica: Number(handles[3]),
				views: Number(handles[4]),
			};
		entry.inline = entry.details.find((d) => d.startsWith("{"));
	}
	return entries;
}
const showPaths = (paths) => `[${paths.map((p) => `{ ${p.map((s) => JSON.stringify(s)).join(", ")} }`).join(", ")}]`;
// Paths differ only in how a map is spelled: the model writes [key], the transformer .key and .value.
const normalPath = (at) => at.replaceAll("[key]", "").replace(/\.(key|value)(?=$|[.[])/g, "");
function reported(result, name) {
	return result.diagnostics
		.map((d) => /^(CRU\d{3}): (\S+) (\S+?): /.exec(d.messageText))
		.filter((m) => m && m[2] === name)
		.map(([, code, , at]) => ({ code, at }));
}
module.exports = {
	...harness,
	project,
	calls,
	diagnosticsIn,
	evaluate,
	describe,
	countNodes,
	verdicts,
	showPaths,
	normalPath,
	reported,
	fixture,
	N,
	S,
	F,
	B,
	Nil,
	L,
	U,
	A,
	M,
	O,
	V,
};
