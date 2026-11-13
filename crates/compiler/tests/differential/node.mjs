// Runs generated programs as TypeScript with Node's type stripping, so nothing of roblox-ts or Crucible takes part.
// Usage: node node.mjs <folder> <name>... Each program is <folder>/<name>.ts. Prints `#<name>` and then one line per
// trace call, in the format README.md states, which lune.luau prints the same way.
import { pathToFileURL } from "node:url";

// The roblox-ts methods JavaScript lacks, as compiler-types declares them. Everything else is JavaScript's own.
function method(prototype, name, value) {
	Object.defineProperty(prototype, name, { value, writable: true, configurable: true });
}
method(Array.prototype, "size", function () {
	return this.length;
});
method(Array.prototype, "isEmpty", function () {
	return this.length === 0;
});
method(Array.prototype, "clear", function () {
	this.length = 0;
});
method(String.prototype, "size", function () {
	return this.length;
});
method(String.prototype, "upper", function () {
	return this.toUpperCase();
});
method(String.prototype, "lower", function () {
	return this.toLowerCase();
});
method(String.prototype, "rep", function (n) {
	return n > 0 ? this.repeat(n) : "";
});
for (const prototype of [Map.prototype, Set.prototype]) {
	const size = Object.getOwnPropertyDescriptor(prototype, "size").get;
	method(prototype, "size", function () {
		return size.call(this);
	});
	method(prototype, "isEmpty", function () {
		return size.call(this) === 0;
	});
}

function number(n) {
	if (Number.isNaN(n)) return "NaN";
	if (n === Infinity) return "Infinity";
	if (n === -Infinity) return "-Infinity";
	if (n === 0) return Object.is(n, -0) ? "-0" : "0";
	if (Number.isInteger(n) && Math.abs(n) < 2 ** 53) return String(n);
	return shortest(n);
}

// The fewest digits that read back as `n`, which `String` gives, written as `d.ddde<exponent>`.
function shortest(n) {
	const sign = n < 0 ? "-" : "";
	let [decimal, exponent] = String(Math.abs(n)).split("e");
	let [whole, fraction = ""] = decimal.split(".");
	let digits = whole + fraction;
	let power = Number(exponent ?? 0) + whole.length - 1;
	const zeros = digits.match(/^0*/)[0].length;
	digits = digits.slice(zeros).replace(/0+$/, "");
	power -= zeros;
	const mantissa = digits.length > 1 ? `${digits[0]}.${digits.slice(1)}` : digits;
	return `${sign}${mantissa}e${power}`;
}

function string(s) {
	let out = '"';
	for (const c of s) {
		const code = c.codePointAt(0);
		if (code >= 0x20 && code < 0x7f && c !== '"' && c !== "\\") out += c;
		else if (code < 0x80) out += `\\x${code.toString(16).padStart(2, "0")}`;
		else out += `\\u{${code.toString(16)}}`;
	}
	return `${out}"`;
}

function dynamic(value) {
	switch (typeof value) {
		case "number":
			return number(value);
		case "string":
			return string(value);
		case "boolean":
			return String(value);
		case "undefined":
			return "undefined";
		default:
			return value === null ? "null" : "<object>";
	}
}

function parse(shape) {
	let at = 0;
	const next = () => shape[at++];
	const expect = (c) => {
		if (next() !== c) throw new Error(`bad shape ${shape} at ${at}`);
	};
	function one() {
		const c = next();
		switch (c) {
			case "n":
			case "s":
			case "b":
			case "x":
				return { kind: c };
			case "?":
				return { kind: "?", inner: one() };
			case "[": {
				const inner = one();
				expect("]");
				return { kind: "[", inner };
			}
			case "S": {
				expect("(");
				const inner = one();
				expect(")");
				return { kind: "S", inner };
			}
			case "M": {
				expect("(");
				const key = one();
				expect(",");
				const value = one();
				expect(")");
				return { kind: "M", key, value };
			}
			case "{": {
				const fields = [];
				while (shape[at] !== "}") {
					let name = "";
					while (shape[at] !== ":") name += next();
					expect(":");
					fields.push([name, one()]);
					if (shape[at] === ",") at++;
				}
				expect("}");
				return { kind: "{", fields };
			}
			default:
				throw new Error(`bad shape ${shape} at ${at}`);
		}
	}
	const parsed = one();
	if (at !== shape.length) throw new Error(`bad shape ${shape}`);
	return parsed;
}

function sorted(items) {
	return items.sort((a, b) => (a < b ? -1 : a > b ? 1 : 0)).join(",");
}

function show(value, shape) {
	switch (shape.kind) {
		case "n":
			return typeof value === "number" ? number(value) : `<not a number: ${dynamic(value)}>`;
		case "s":
			return typeof value === "string" ? string(value) : `<not a string: ${dynamic(value)}>`;
		case "b":
			return typeof value === "boolean" ? String(value) : `<not a boolean: ${dynamic(value)}>`;
		case "x":
			return dynamic(value);
		case "?":
			return value === undefined ? "undefined" : show(value, shape.inner);
		case "[": {
			if (!Array.isArray(value)) return `<not an array: ${dynamic(value)}>`;
			const items = [];
			for (let i = 0; i < value.length; i++) items.push(show(value[i], shape.inner));
			return `[${items.join(",")}]`;
		}
		case "{":
			if (typeof value !== "object" || value === null) return `<not an object: ${dynamic(value)}>`;
			return `{${shape.fields.map(([name, field]) => `${name}=${show(value[name], field)}`).join(",")}}`;
		case "M":
			if (!(value instanceof Map)) return `<not a map: ${dynamic(value)}>`;
			return `M{${sorted([...value].map(([k, v]) => `${show(k, shape.key)}=>${show(v, shape.value)}`))}}`;
		case "S":
			if (!(value instanceof Set)) return `<not a set: ${dynamic(value)}>`;
			return `S{${sorted([...value].map((v) => show(v, shape.inner)))}}`;
	}
}

const shapes = new Map();
function trace(lines) {
	return (value, shape) => {
		let parsed = shapes.get(shape);
		if (parsed === undefined) {
			parsed = parse(shape);
			shapes.set(shape, parsed);
		}
		lines.push(show(value, parsed));
	};
}

const [folder, ...names] = process.argv.slice(2);
let out = "";
for (const name of names) {
	const lines = [];
	globalThis.trace = trace(lines);
	try {
		await import(pathToFileURL(`${folder}/${name}.ts`).href);
	} catch (error) {
		lines.push(
			error instanceof Error ? `!error ${string(`${error.name}: ${error.message}`)}` : `!throw ${dynamic(error)}`,
		);
	}
	out += `#${name}\n`;
	for (const line of lines) out += `${line}\n`;
}
process.stdout.write(out);
