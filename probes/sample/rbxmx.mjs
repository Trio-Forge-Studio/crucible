const TOKEN = /<!\[CDATA\[[\s\S]*?\]\]>|<[^>]+>|[^<]+/g;

function unescape(value) {
	return value
		.replaceAll("&lt;", "<")
		.replaceAll("&gt;", ">")
		.replaceAll("&quot;", '"')
		.replaceAll("&apos;", "'")
		.replaceAll("&amp;", "&");
}

// Reads a Rojo-built .rbxmx into { class, name, source?, value?, runContext?, children } nodes. Only the properties
// Crucible's sample needs are kept: Name, Source, Value and RunContext.
export function parseRbxmx(xml) {
	const stack = [];
	let root;
	let property;
	for (const [token] of xml.matchAll(TOKEN)) {
		if (token.startsWith("<![CDATA[")) {
			if (property) property.text += token.slice(9, -3);
		} else if (token.startsWith("<")) {
			const item = /^<Item class="([^"]+)"/.exec(token);
			const open = /^<(string|ProtectedString|token) name="([^"]+)">$/.exec(token);
			if (item) {
				const node = { class: item[1], name: "", children: [] };
				if (stack.length > 0) stack[stack.length - 1].children.push(node);
				else root = node;
				stack.push(node);
			} else if (token === "</Item>") stack.pop();
			else if (open) property = { type: open[1], name: open[2], text: "" };
			else if (/^<\/(string|ProtectedString|token)>$/.test(token) && property) {
				const node = stack[stack.length - 1];
				const text = property.type === "ProtectedString" ? property.text : unescape(property.text);
				if (property.name === "Name") node.name = text;
				else if (property.name === "Source") node.source = text;
				else if (property.name === "Value") node.value = text;
				else if (property.name === "RunContext") node.runContext = Number(text);
				property = undefined;
			}
		} else if (property) property.text += token;
	}
	return root;
}

function longBracket(text) {
	let level = 0;
	while (text.includes(`]${"=".repeat(level)}]`)) level++;
	const bar = "=".repeat(level);
	return `[${bar}[\n${text}]${bar}]`;
}

// Instance-creation steps for every descendant of `root`, parents first. A step's `path` names its parent below the
// session's DataModel, so a chunk can find it again. `skip(path, node)` leaves a subtree out.
export function steps(root, parentPath, skip = () => false) {
	const out = [];
	(function walk(node, path) {
		for (const child of node.children) {
			if (skip(path, child)) continue;
			out.push({ path, node: child });
			walk(child, [...path, child.name]);
		}
	})(root, parentPath);
	return out;
}

function code(step) {
	const { node, path } = step;
	const lines = [`i = Instance.new(${JSON.stringify(node.class)})`, `i.Name = ${JSON.stringify(node.name)}`];
	if (node.source !== undefined) lines.push(`i.Source = ${longBracket(node.source)}`);
	if (node.value !== undefined) lines.push(`i.Value = ${JSON.stringify(node.value)}`);
	if (node.runContext !== undefined) lines.push(`i.RunContext = Enum.RunContext:FromValue(${node.runContext})`);
	lines.push(`i.Parent = at({ ${path.map((name) => JSON.stringify(name)).join(", ")} })`);
	return lines.join("\n");
}

// Luau chunks of at most `limit` characters, each creating its steps under `game`.
export function chunks(list, limit) {
	const head = `local function at(path) local node = game for _, name in path do node = assert(node:FindFirstChild(name), name) end return node end\nlocal i, _\n`;
	const out = [];
	let body = "";
	for (const step of list) {
		const piece = code(step) + "\n";
		if (body !== "" && head.length + body.length + piece.length > limit) {
			out.push(head + body + "return 1");
			body = "";
		}
		body += piece;
	}
	if (body !== "") out.push(head + body + "return 1");
	return out;
}
