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
				types: (node.typeArguments ?? []).map((t) => t.getText(sf)),
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
module.exports = { ...harness, project, calls, diagnosticsIn, N, S, F, B, Nil, L, U, A, M, O, V };
