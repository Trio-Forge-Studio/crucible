import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { createRequire } from "node:module";
const require = createRequire(import.meta.url);
const repo = process.cwd();
const scratch = fs.mkdtempSync(path.join(os.tmpdir(), "crucible-migrations-mutants-"));
const D = "packages/crucible/src/internal/data/";
const A = D + "admission.ts";
const RC = D + "records.ts";
const RD = D + "record-declarations.ts";
const RB = D + "record-bindings.ts";
const P = D + "profiles.ts";
const S = D + "session.ts";
const I = D + "inbox.ts";
const O = D + "operation-id.ts";
const ADM = "data-admission.luau";
const MIG = "data-migrations.luau";
const UNK = "data-unknown-keys.luau";
const REC = "data-records.luau";
const LIM = "data-records-limits.luau";
const m = (rule, kind, file, before, after, spec) => [rule, kind, file, before, after, spec];
// A pattern ignores how deep its lines are indented, so a row names the code and not the nesting around it.
const matcher = (before) =>
	new RegExp(
		before
			.split("\n")
			.map((line) => line.replace(/^\t*/, "").replace(/[.*+?^${}()|[\]\\]/g, "\\$&"))
			.map((line) => `\\t*${line}`)
			.join("\n"),
		"g",
	);
const occurrences = (text, before) => (text.match(matcher(before)) ?? []).length;
const apply = (text, before, after) => text.replace(matcher(before), () => after);
// K3 (ruling K3): every rule with a site has a guard, a swap and a pair mutant below. A rule or site with no such
// operator is listed here with the reason. A spec target is a spec file or a case-name fragment; an array runs several.
const K3 = [
	"rule 1 swap and pair: installed() is one guard on one module variable, with no operands to exchange and no second half",
	"rule 2 pair: the name length test is one comparison, with no second half",
	"rule 13 swap: dataCallback passes one worker, one record and one undefined, so no two operands share a type",
	"rule 49 guard and swap: the only D3 site in inspect() is its loadRecord call (a pair row); the ok-or-unavailable ternary cannot type-check with its arms exchanged",
	"rules 69, 71 and 72 swap and pair: amendment A16 names one guard or boundary site each in inbox.ts and operation-id.ts, and each has its row",
	"rule 70 pair: the quarantine-full diagnostic is one guarded call with no second half; its swap is the attempts operand row",
	"rule 77 guard: the newer branch of ProfileRuntime.load is unreachable (lease acquire refuses a newer stored version first), so no guard there can be observed",
	"rule 110 pair: a diagnostic key and its message are one call, with no second half",
	"rule 15 equivalent: the known() reuse of a frozen admitted table in object, array, map and json (and the admitted.set entry that feeds it) only skips a re-walk that returns the same table at the same cost",
	"rule 15 equivalent: `!table.isfrozen(value)` in known() is unreachable, finish() only caches a frozen table and a frozen table cannot thaw",
	"rule 76 equivalent: `length > 0` in json() sends an empty table to the object branch, which builds the same empty copy",
	"rule 76 equivalent: the typeIs(key, string) test in the map loop gives the same path and reason as the hasOnlyStringKeys test before it, and the two differ only in which of two independent failures is reported first",
	"rule 110 equivalent: a diagnostic key is only compared for equality, so exchanging the store and the code (or the user) in it partitions the diagnostics the same way",
	"rule 76 equivalent: the discriminant of a variant is copied and counted once either by the declared pass or by the unknown-key pass",
	"rule 76 equivalent: the integer, lower-bound and upper-bound tests on a key in denseLength differ from the count test only where Luau's length operator returns a border past an interior hole, which depends on table layout and has no stable input",
	"rule 14 equivalent: the rethrow of a non-admission error in admit() is unreachable, no input raises one",
	"rule 74 equivalent: `step === undefined` in loadRecord is unreachable, a schema's version is its step count (rule 74)",
	"rule 77 equivalent: the version carried by a newer Loaded and by the newer decision of updateRecord is never read where it can be reached",
	"rule 87 equivalent: the `: newer` reason in updateRecord is unreachable, the newer test above it has already returned",
	"rule 4 equivalent: sealSchema returning `node` is the same as leaving it off, every reader falls back to the same open node",
	"rule 88 equivalent: ageOf's malformed result and the empty-seconds pattern are unreachable, createdAt is checked first and tonumber of an empty string is nil",
	"rule 104 equivalent: bound.set keyed by the declaration's name is the same string as the map key",
];
// [rule, kind, file, before, after, spec]. Kinds: guard deletes a reset or guard, swap exchanges two values of one
// type, pair short-circuits one half of a paired operation.
const mutants = [
	// Admission walk: rules 14, 15 and 76.
	m(
		"15",
		"guard",
		A,
		"hit === undefined || hit.node !== node || !table.isfrozen(value)",
		"hit === undefined || !table.isfrozen(value)",
		["data-admission.luau: rule 15 a frozen table admitted under one descriptor is admitted again under another"],
	),
	m("15", "pair", A, "\t\tvisits += hit.visits;\n", "", [
		"data-admission.luau: rule 15 a cached array counts its full visit cost at every use",
	]),
	m(
		"15",
		"guard",
		A,
		"\t\tvisits += hit.visits;\n\t\tif (visits > PERSIST_ADMISSION_VISITS) fail(path, `more than ${PERSIST_ADMISSION_VISITS} values`);\n",
		"\t\tvisits += hit.visits;\n",
		["data-admission.luau: rule 15 a cached array counts its full visit cost at every use"],
	),
	m(
		"15",
		"guard",
		A,
		"visits += hit.visits;\n\t\tif (visits > PERSIST_ADMISSION_VISITS)",
		"visits += hit.visits;\n\t\tif (visits >= PERSIST_ADMISSION_VISITS)",
		["data-admission.luau: rule 15 the visit limit is exact for tables reused from the current value"],
	),
	m(
		"15",
		"guard",
		A,
		"\t\tif (depth + hit.height - 1 > PERSIST_DEPTH) fail(path, `deeper than ${PERSIST_DEPTH}`);\n",
		"",
		["data-admission.luau: rule 15 a frozen subtree reused where its deepest table passes depth 28 is refused"],
	),
	m("15", "guard", A, "depth + hit.height - 1 > PERSIST_DEPTH", "depth + hit.height > PERSIST_DEPTH", [
		"data-admission.luau: rule 15 a frozen subtree reused where its deepest table passes depth 28 is refused",
	]),
	m("15", "guard", A, "depth + hit.height - 1 > PERSIST_DEPTH", "depth + hit.height - 2 > PERSIST_DEPTH", [
		"data-admission.luau: rule 15 a frozen subtree reused where its deepest table passes depth 28 is refused",
	]),
	m("15", "pair", A, "\t\theight = hit.height;\n\t\treturn true;", "\t\treturn true;", [
		"data-admission.luau: rule 15 a table rebuilt around a cached subtree carries the subtree's depth",
	]),
	m(
		"15",
		"guard",
		A,
		"same && table.isfrozen(source) ? source : table.freeze(copy)",
		"table.isfrozen(source) ? source : table.freeze(copy)",
		["data-admission.luau: rule 15 a frozen table that holds an unfrozen one is copied and frozen throughout"],
	),
	m(
		"15",
		"guard",
		A,
		"same && table.isfrozen(source) ? source : table.freeze(copy)",
		"same ? source : table.freeze(copy)",
		["data-admission.luau: rule 15 a frozen subtree of the current value is reused by identity when written back"],
	),
	m(
		"15",
		"swap",
		A,
		"same && table.isfrozen(source) ? source : table.freeze(copy)",
		"same && table.isfrozen(source) ? table.freeze(copy) : source",
		["data-admission.luau: rule 15 a frozen subtree of the current value is reused by identity when written back"],
	),
	m(
		"15",
		"swap",
		A,
		"admitted.set(result, { node, visits: visits - before, height });",
		"admitted.set(result, { node, visits: height, height: visits - before });",
		["data-admission.luau: rule 15 a cached array counts its full visit cost at every use"],
	),
	m("15", "guard", A, "\t\theight = top + 1;\n", "\t\theight = top;\n", [
		"data-admission.luau: rule 15 a frozen subtree reused where its deepest table passes depth 28 is refused",
	]),
	m("15", "guard", A, "\t\theight = top + 1;\n", "\t\theight = top + 2;\n", [
		"data-admission.luau: rule 15 a frozen subtree reused where its deepest table passes depth 28 is refused",
	]),
	m(
		"15",
		"swap",
		A,
		"copy.push(result as defined);\n\t\t\t\t}\n\t\t\t\treturn finish(source, copy, node, before, top, same);",
		"copy.push(result as defined);\n\t\t\t\t}\n\t\t\t\treturn finish(source, copy, node, top, before, same);",
		["data-admission.luau: rule 15 the visit limit is exact for tables reused from the current value"],
	),
	m(
		"15",
		"swap",
		A,
		"copy[key as string] = child;\n\t\t\t}\n\t\t}\n\t\treturn finish(value, copy, JSON_NODE, before, top, same);",
		"copy[key as string] = child;\n\t\t\t}\n\t\t}\n\t\treturn finish(value, copy, JSON_NODE, top, before, same);",
		["data-admission.luau: rule 15 a frozen subtree reused where its deepest table passes depth 28 is refused"],
	),
	m("15", "pair", A, "\t\topen.delete(source);\n", "", [
		"data-admission.luau: rule 14 a table shared by two fields is not a cycle",
	]),
	m("14", "pair", A, "\t\topen.add(value);\n", "", [
		"data-admission.luau: rule 76 a table with a metatable and a cycle are refused at the engine boundary",
	]),
	m("14", "guard", A, '\t\tif (open.has(value)) fail(path, "a cycle");\n', "", [
		"data-admission.luau: rule 76 a table with a metatable and a cycle are refused at the engine boundary",
	]),
	m("14", "swap", A, 'fail(path, "a cycle");', 'fail("a cycle", path);', [
		"data-admission.luau: rule 76 a table with a metatable and a cycle are refused at the engine boundary",
	]),
	m("14", "guard", A, '\t\tif (getmetatable(value) !== undefined) fail(path, "a table with a metatable");\n', "", [
		"data-admission.luau: rule 76 a table with a metatable and a cycle are refused at the engine boundary",
	]),
	m(
		"14",
		"guard",
		A,
		"\t\tif (depth > PERSIST_DEPTH) fail(path, `deeper than ${PERSIST_DEPTH}`);\n\t\tif (getmetatable",
		"\t\tif (getmetatable",
		["data-admission.luau: rule 76 depth 28 loads and depth 29 does not, in objects and in arrays"],
	),
	m("14", "guard", A, "if (depth > PERSIST_DEPTH) fail(path,", "if (depth >= PERSIST_DEPTH) fail(path,", [
		"data-admission.luau: rule 76 depth 28 loads and depth 29 does not, in objects and in arrays",
	]),
	m("14", "pair", A, "\t\tcount(path);\n\t\tif (depth > PERSIST_DEPTH)", "\t\tif (depth > PERSIST_DEPTH)", [
		"data-admission.luau: rule 76 100000 values load and 100001 do not, counting the root and every table",
	]),
	m("14", "pair", A, "\t\tvisits += 1;\n", "", [
		"data-admission.luau: rule 15 a cached array counts its full visit cost at every use",
	]),
	m(
		"14",
		"guard",
		A,
		"\t\tvisits += 1;\n\t\tif (visits > PERSIST_ADMISSION_VISITS) fail(path, `more than ${PERSIST_ADMISSION_VISITS} values`);\n",
		"\t\tvisits += 1;\n",
		["data-admission.luau: rule 76 100000 values load and 100001 do not, counting the root and every table"],
	),
	m(
		"14",
		"guard",
		A,
		"visits += 1;\n\t\tif (visits > PERSIST_ADMISSION_VISITS)",
		"visits += 1;\n\t\tif (visits >= PERSIST_ADMISSION_VISITS)",
		["data-admission.luau: rule 76 100000 values load and 100001 do not, counting the root and every table"],
	),
	m(
		"14",
		"swap",
		A,
		"fail(path, `more than ${PERSIST_ADMISSION_VISITS} values`);\n\t}\n\n\tfunction enter",
		"fail(`more than ${PERSIST_ADMISSION_VISITS} values`, path);\n\t}\n\n\tfunction enter",
		["data-admission.luau: rule 76 100000 values load and 100001 do not, counting the root and every table"],
	),
	m(
		"14",
		"swap",
		A,
		"fail(path, `deeper than ${PERSIST_DEPTH}`);\n\t\tif (getmetatable",
		"fail(`deeper than ${PERSIST_DEPTH}`, path);\n\t\tif (getmetatable",
		["data-admission.luau: rule 76 depth 28 loads and depth 29 does not, in objects and in arrays"],
	),
	// Scalars.
	m(
		"76",
		"guard",
		A,
		'\t\tif (!typeIs(value, "string")) fail(path, `expected string, got ${typeOf(value)}`);\n',
		'\t\tif (!typeIs(value, "string")) return value as never;\n',
		[
			"data-admission.luau: rule 76 a present declared value of the wrong shape or range makes the load fail with the path, the data untouched",
		],
	),
	m(
		"76",
		"swap",
		A,
		"fail(path, `expected string, got ${typeOf(value)}`)",
		"fail(`expected string, got ${typeOf(value)}`, path)",
		[
			"data-admission.luau: rule 76 a present declared value of the wrong shape or range makes the load fail with the path, the data untouched",
		],
	),
	m(
		"76",
		"guard",
		A,
		"\t\tif (value.size() > PERSIST_STRING_BYTES) fail(path, `a string over ${PERSIST_STRING_BYTES} bytes`);\n",
		"",
		[
			"data-admission.luau: rule 14 and 76 a write over PAYLOAD_MAX_BYTES throws naming the store and user, a single string over 65536 is refused first",
		],
	),
	m("76", "guard", A, "value.size() > PERSIST_STRING_BYTES", "value.size() >= PERSIST_STRING_BYTES", [
		"data-admission.luau: rule 76 the width and length boundaries themselves are accepted, and nothing is coerced",
	]),
	m("76", "guard", A, '\t\tif (utf8.len(value)[0] === undefined) fail(path, "invalid UTF-8");\n', "", [
		"data-admission.luau: rule 76 a present declared value of the wrong shape or range makes the load fail with the path, the data untouched",
	]),
	m(
		"76",
		"swap",
		A,
		"fail(path, `a string over ${PERSIST_STRING_BYTES} bytes`)",
		"fail(`a string over ${PERSIST_STRING_BYTES} bytes`, path)",
		[
			"data-admission.luau: rule 14 and 76 a write over PAYLOAD_MAX_BYTES throws naming the store and user, a single string over 65536 is refused first",
		],
	),
	m(
		"76",
		"guard",
		A,
		'\t\tif (!typeIs(value, "number")) fail(path, `expected number, got ${typeOf(value)}`);\n',
		'\t\tif (!typeIs(value, "number")) return value as never;\n',
		[
			"data-admission.luau: rule 76 a present declared value of the wrong shape or range makes the load fail with the path, the data untouched",
		],
	),
	m(
		"76",
		"swap",
		A,
		"fail(path, `expected number, got ${typeOf(value)}`)",
		"fail(`expected number, got ${typeOf(value)}`, path)",
		[
			"data-admission.luau: rule 76 a present declared value of the wrong shape or range makes the load fail with the path, the data untouched",
		],
	),
	m("76", "guard", A, "value !== value || math.abs(value) === math.huge", "math.abs(value) === math.huge", [
		"data-admission.luau: rule 76 a present declared value of the wrong shape or range makes the load fail with the path, the data untouched",
	]),
	m("76", "guard", A, "value !== value || math.abs(value) === math.huge", "value !== value", [
		"data-admission.luau: rule 76 a present declared value of the wrong shape or range makes the load fail with the path, the data untouched",
	]),
	m(
		"76",
		"guard",
		A,
		'\t\tif (value !== value || math.abs(value) === math.huge) fail(path, "expected a finite number");\n',
		"",
		[
			"data-admission.luau: rule 76 a present declared value of the wrong shape or range makes the load fail with the path, the data untouched",
		],
	),
	m(
		"76",
		"guard",
		A,
		'\t\t\t\tif (!typeIs(value, "boolean")) fail(path, `expected boolean, got ${typeOf(value)}`);\n',
		"",
		[
			"data-admission.luau: rule 110 unreadable-record diagnostics carry store, user and path, never a value, and a user who rejoins is logged once",
		],
	),
	m(
		"76",
		"swap",
		A,
		"fail(path, `expected boolean, got ${typeOf(value)}`)",
		"fail(`expected boolean, got ${typeOf(value)}`, path)",
		[
			"data-admission.luau: rule 14 a write whose result has a wrong declared value throws at the call with the store and path, never the value, and changes nothing",
		],
	),
	m("76", "guard", A, "\t\t\t\tif (value !== node.value) fail(path, `expected ${describe(node)}`);\n", "", [
		"data-admission.luau: rule 76 a present declared value of the wrong shape or range makes the load fail with the path, the data untouched",
	]),
	m("76", "swap", A, "fail(path, `expected ${describe(node)}`);", "fail(`expected ${describe(node)}`, path);", [
		"data-admission.luau: rule 76 a present declared value of the wrong shape or range makes the load fail with the path, the data untouched",
	]),
	m(
		"76",
		"guard",
		A,
		'\t\t\tcase "nil":\n\t\t\t\tfail(path, "expected nothing");\n',
		'\t\t\tcase "nil":\n\t\t\t\treturn scalar(value, path);\n',
		[
			"data-admission.luau: rule 76 a present declared value of the wrong shape or range makes the load fail with the path, the data untouched",
		],
	),
	m(
		"76",
		"guard",
		A,
		'if (math.abs(number(value, path)) > F32_MAX) fail(path, "out of f32 range");',
		"number(value, path);",
		[
			"data-admission.luau: rule 76 a present declared value of the wrong shape or range makes the load fail with the path, the data untouched",
		],
	),
	m("76", "guard", A, "math.abs(number(value, path)) > F32_MAX", "math.abs(number(value, path)) >= F32_MAX", [
		"data-admission.luau: rule 76 the width and length boundaries themselves are accepted, and nothing is coerced",
	]),
	m("76", "guard", A, "amount % 1 !== 0 || amount < 0 || amount > maximum", "amount < 0 || amount > maximum", [
		"data-admission.luau: rule 76 a present declared value of the wrong shape or range makes the load fail with the path, the data untouched",
	]),
	m("76", "guard", A, "amount % 1 !== 0 || amount < 0 || amount > maximum", "amount % 1 !== 0 || amount > maximum", [
		"data-admission.luau: rule 76 a present declared value of the wrong shape or range makes the load fail with the path, the data untouched",
	]),
	m("76", "guard", A, "amount % 1 !== 0 || amount < 0 || amount > maximum", "amount % 1 !== 0 || amount < 0", [
		"data-admission.luau: rule 76 a present declared value of the wrong shape or range makes the load fail with the path, the data untouched",
	]),
	m(
		"76",
		"guard",
		A,
		"amount % 1 !== 0 || amount < 0 || amount > maximum",
		"amount % 1 !== 0 || amount < 0 || amount >= maximum",
		[
			"data-admission.luau: rule 76 the width and length boundaries themselves are accepted, and nothing is coerced",
		],
	),
	m(
		"76",
		"swap",
		A,
		"const UINT_MAX = { u8: 255, u16: 65_535, u32: 4_294_967_295 };",
		"const UINT_MAX = { u8: 65_535, u16: 255, u32: 4_294_967_295 };",
		[
			"data-admission.luau: rule 76 a present declared value of the wrong shape or range makes the load fail with the path, the data untouched",
		],
	),
	m(
		"76",
		"swap",
		A,
		"const UINT_MAX = { u8: 255, u16: 65_535, u32: 4_294_967_295 };",
		"const UINT_MAX = { u8: 255, u16: 4_294_967_295, u32: 65_535 };",
		[
			"data-admission.luau: rule 76 a present declared value of the wrong shape or range makes the load fail with the path, the data untouched",
		],
	),
	m(
		"76",
		"swap",
		A,
		"fail(path, `expected an integer in [0, ${maximum}]`)",
		"fail(`expected an integer in [0, ${maximum}]`, path)",
		[
			"data-admission.luau: rule 76 a present declared value of the wrong shape or range makes the load fail with the path, the data untouched",
		],
	),
	// Objects, defaults and unknown keys.
	m("76", "guard", A, '\t\tif (!hasOnlyStringKeys(source)) fail(path, "expected an object");\n', "", [
		"data-admission.luau: rule 76 a present declared value of the wrong shape or range makes the load fail with the path, the data untouched",
	]),
	m(
		"76",
		"swap",
		A,
		'const defaults = typeIs(fallback, "table") ? (fallback as Record<string, unknown>) : undefined;',
		'const defaults = typeIs(fallback, "table") ? undefined : (fallback as Record<string, unknown>);',
		[
			"data-admission.luau: rule 76 R5 an absent optional object stays absent and a stored one takes the default at its path only when defaults hold one",
		],
	),
	m("76", "pair", A, "\t\t\tdeclared.add(name);\n", "", [
		"data-admission.luau: rule 76 100000 values load and 100001 do not, counting the root and every table",
	]),
	m("76", "pair", A, "\t\t\t\tdeclared.add(discriminant);\n", "", [
		"data-admission.luau: rule 76 variant objects count their tag once, at exactly 100,000 visits",
	]),
	m("76", "pair", A, "\t\t\t\tcopy[discriminant] = source[discriminant];\n", "", [
		"data-admission.luau: rule 14 a write whose result has a wrong declared value throws at the call with the store and path, never the value, and changes nothing",
	]),
	m("76", "pair", A, "\t\t\t\tcount(`${path}.${discriminant}`);\n", "", [
		"data-admission.luau: rule 76 variant objects count their tag once, at exactly 100,000 visits",
	]),
	m("76", "guard", A, "\t\t\t\tif (admitsNil(child)) continue;\n", "", [
		"data-admission.luau: rule 76 R5 an absent optional object stays absent and a stored one takes the default at its path only when defaults hold one",
	]),
	m(
		"76",
		"guard",
		A,
		'\t\t\t\tif (fieldDefault === undefined)\n\t\t\t\t\tfail(fieldPath, "missing; a field added to existing data needs a migration step that fills it");\n',
		"",
		[
			"data-admission.luau: rule 14 a write whose result has a wrong declared value throws at the call with the store and path, never the value, and changes nothing",
		],
	),
	m(
		"76",
		"swap",
		A,
		'fail(fieldPath, "missing; a field added to existing data needs a migration step that fills it")',
		'fail("missing; a field added to existing data needs a migration step that fills it", fieldPath)',
		[
			"data-admission.luau: rule 14 a write whose result has a wrong declared value throws at the call with the store and path, never the value, and changes nothing",
		],
	),
	m("76", "pair", A, "\t\t\t\tcopy[name] = fieldDefault;\n\t\t\t\tsame = false;\n", "\t\t\t\tsame = false;\n", [
		"data-admission.luau: rule 76 R5 an absent optional object stays absent and a stored one takes the default at its path only when defaults hold one",
	]),
	m(
		"76",
		"pair",
		A,
		"\t\t\t\tcopy[name] = fieldDefault;\n\t\t\t\tsame = false;\n",
		"\t\t\t\tcopy[name] = fieldDefault;\n",
		["data-migrations.luau: rule 76 a default fills a field missing from a frozen table a migration step returned"],
	),
	m(
		"76",
		"swap",
		A,
		"const result = walk(child, field, fieldDefault, fieldPath, depth + 1);",
		"const result = walk(child, fieldDefault, field, fieldPath, depth + 1);",
		[
			"data-admission.luau: rule 110 unreadable-record diagnostics carry store, user and path, never a value, and a user who rejoins is logged once",
		],
	),
	m("76", "guard", A, "\t\t\t\tif (!rawequal(result, field)) same = false;\n", "", [
		"data-admission.luau: rule 15 a frozen table that holds an unfrozen one is copied and frozen throughout",
	]),
	m("78", "guard", A, "\t\t\tif (declared.has(key as string)) continue;\n", "", [
		"data-admission.luau: rule 76 100000 values load and 100001 do not, counting the root and every table",
	]),
	m(
		"78",
		"swap",
		A,
		"const result = json(entry, `${path}.${key}`, depth + 1);\n\t\t\ttop = math.max(top, height);\n\t\t\tif (!rawequal(result, entry)) same = false;\n\t\t\tcopy[key as string] = result;",
		"const result = json(entry, `${path}.${key}`, depth + 1);\n\t\t\ttop = math.min(top, height);\n\t\t\tif (!rawequal(result, entry)) same = false;\n\t\t\tcopy[key as string] = result;",
		["data-admission.luau: rule 15 a declared node reused deeper carries the depth of its unknown keys"],
	),
	m(
		"78",
		"pair",
		A,
		"\t\t\tif (!rawequal(result, entry)) same = false;\n\t\t\tcopy[key as string] = result;",
		"\t\t\tcopy[key as string] = result;",
		[ADM, UNK],
	),
	m(
		"78",
		"pair",
		A,
		"\t\t\tconst result = json(entry, `${path}.${key}`, depth + 1);\n\t\t\ttop = math.max(top, height);\n\t\t\tif (!rawequal(result, entry)) same = false;\n\t\t\tcopy[key as string] = result;",
		"\t\t\tconst result = entry;\n\t\t\ttop = math.max(top, height);\n\t\t\tif (!rawequal(result, entry)) same = false;\n\t\t\tcopy[key as string] = result;",
		[
			"data-admission.luau: rule 14 and 76 a write over PAYLOAD_MAX_BYTES throws naming the store and user, a single string over 65536 is refused first",
		],
	),
	m(
		"78",
		"swap",
		A,
		"json(entry, `${path}.${key}`, depth + 1);\n\t\t\ttop = math.max(top, height);\n\t\t\tif (!rawequal(result, entry)) same = false;\n\t\t\tcopy[key as string] = result;",
		"json(entry, `${path}.${key}`, depth);\n\t\t\ttop = math.max(top, height);\n\t\t\tif (!rawequal(result, entry)) same = false;\n\t\t\tcopy[key as string] = result;",
		["data-admission.luau: rule 15 a frozen subtree reused where its deepest table passes depth 28 is refused"],
	),
	// Arrays, tuples and maps.
	m(
		"76",
		"guard",
		A,
		'\t\t\t\tif (length === undefined) fail(path, "expected an array");\n',
		"\t\t\t\tif (length === undefined) return source as never;\n",
		[
			"data-admission.luau: rule 76 a present declared value of the wrong shape or range makes the load fail with the path, the data untouched",
		],
	),
	m(
		"76",
		"guard",
		A,
		'node.kind === "tuple" && length !== node.elements.size()',
		'node.kind === "tuple" && (false as boolean)',
		[
			"data-admission.luau: rule 76 a present declared value of the wrong shape or range makes the load fail with the path, the data untouched",
		],
	),
	m(
		"76",
		"swap",
		A,
		"fail(path, `expected ${node.elements.size()} elements`)",
		"fail(`expected ${node.elements.size()} elements`, path)",
		[
			"data-admission.luau: rule 76 a present declared value of the wrong shape or range makes the load fail with the path, the data untouched",
		],
	),
	m(
		"76",
		"swap",
		A,
		'const element = node.kind === "tuple" ? node.elements[i] : node.element;',
		'const element = node.kind === "tuple" ? node.elements[math.max(i - 1, 0)] : node.element;',
		[
			"data-admission.luau: rule 76 a present declared value of the wrong shape or range makes the load fail with the path, the data untouched",
		],
	),
	m(
		"76",
		"swap",
		A,
		"walk(element, items[i], undefined, `${path}[${i}]`, depth + 1);",
		"walk(element, undefined, items[i], `${path}[${i}]`, depth + 1);",
		[
			"data-admission.luau: rule 110 unreadable-record diagnostics carry store, user and path, never a value, and a user who rejoins is logged once",
		],
	),
	m(
		"76",
		"guard",
		A,
		'\t\t\t\t\tif (restricted && !keys.has(key)) fail(`${path}.${key}`, "not a key this map declares");\n',
		"",
		[
			"data-admission.luau: rule 76 a present declared value of the wrong shape or range makes the load fail with the path, the data untouched",
		],
	),
	m(
		"76",
		"swap",
		A,
		'fail(`${path}.${key}`, "not a key this map declares")',
		'fail("not a key this map declares", `${path}.${key}`)',
		[
			"data-admission.luau: rule 76 a present declared value of the wrong shape or range makes the load fail with the path, the data untouched",
		],
	),
	m("76", "pair", A, "\t\t\t\t\tfor (const branch of node.key.branches) keys.add(tostring(branch.value));\n", "", [
		"data-admission.luau: rule 76 the width and length boundaries themselves are accepted, and nothing is coerced",
	]),
	m("76", "guard", A, '} else if (node.key.kind !== "string") fail(path, "a saved map has string keys");', "}", [
		"data-admission.luau: rule 76 a map keyed by a number refuses the store at its path when it is declared",
	]),
	m(
		"76",
		"swap",
		A,
		"walk(node.value, entry, undefined, `${path}.${key}`, depth + 1);",
		"walk(node.value, undefined, entry, `${path}.${key}`, depth + 1);",
		[
			"data-admission.luau: rule 110 unreadable-record diagnostics carry store, user and path, never a value, and a user who rejoins is logged once",
		],
	),
	m("76", "pair", A, "\t\t\t\t\tcopy[key] = result;\n", "", [
		"data-admission.luau: rule 76 and 82 a step fills a required key missing in a map value or a non-default variant",
	]),
	// Unions and variants.
	m("76", "guard", A, '\t\t\t\t\tif (branch.kind === "nil") continue;\n', "", [
		"data-admission.luau: rule 110 unreadable-record diagnostics carry store, user and path, never a value, and a user who rejoins is logged once",
	]),
	m("76", "guard", A, "if (value === branch.value) return scalar(value, path);", "return scalar(value, path);", [
		"data-admission.luau: rule 76 a present declared value of the wrong shape or range makes the load fail with the path, the data untouched",
	]),
	m(
		"76",
		"guard",
		A,
		'if (!typeIs(value, "table") || (value as Record<string, unknown>)[branch.key] !== branch.value)\n\t\t\t\t\t\t\tcontinue;',
		'if (!typeIs(value, "table")) continue;',
		["data-admission.luau: rule 76 inside a union only the default's own variant fills a missing key"],
	),
	m(
		"76",
		"swap",
		A,
		"return object(branch.body, value, own ? fallback : undefined, path, depth, branch.key);",
		"return object(branch.body, value, own ? undefined : fallback, path, depth, branch.key);",
		["data-admission.luau: rule 76 inside a union only the default's own variant fills a missing key"],
	),
	m(
		"76",
		"guard",
		A,
		"return object(branch.body, value, own ? fallback : undefined, path, depth, branch.key);",
		"return object(branch.body, value, fallback, path, depth, branch.key);",
		["data-admission.luau: rule 76 inside a union only the default's own variant fills a missing key"],
	),
	m("76", "guard", A, "(fallback as Record<string, unknown>)[branch.key] === branch.value;", "true;", [
		"data-admission.luau: rule 76 inside a union only the default's own variant fills a missing key",
	]),
	m(
		"76",
		"guard",
		A,
		"} else if (runtimeKind(branch) === kind) return walk(branch, value, fallback, path, depth);",
		"} else return walk(branch, value, fallback, path, depth);",
		[
			"data-unknown-keys.luau: rule 76 and 78 undeclared keys count toward the payload cap, depth and visit limits and must be JSON",
		],
	),
	m(
		"76",
		"swap",
		A,
		"} else if (runtimeKind(branch) === kind) return walk(branch, value, fallback, path, depth);",
		"} else if (runtimeKind(branch) === kind) return walk(branch, fallback, value, path, depth);",
		[
			"data-admission.luau: rule 110 unreadable-record diagnostics carry store, user and path, never a value, and a user who rejoins is logged once",
		],
	),
	m(
		"76",
		"swap",
		A,
		"return fail(path, `expected ${describe(node)}, got ${kind}`);",
		"return fail(`expected ${describe(node)}, got ${kind}`, path);",
		[
			"data-admission.luau: rule 76 a present declared value of the wrong shape or range makes the load fail with the path, the data untouched",
		],
	),
	m(
		"76",
		"swap",
		A,
		"return fail(path, `${node.kind} cannot be saved; a save holds JSON`);",
		"return fail(`${node.kind} cannot be saved; a save holds JSON`, path);",
		[
			"data-admission.luau: rule 76 a descriptor kind that is not JSON refuses a default at its path when the store is declared",
		],
	),
	m(
		"76",
		"swap",
		A,
		'\t\t\tcase "object":\n\t\t\tcase "map":\n\t\t\t\treturn "an object";\n\t\t\tcase "array":\n\t\t\tcase "tuple":\n\t\t\t\treturn "an array";',
		'\t\t\tcase "object":\n\t\t\tcase "map":\n\t\t\t\treturn "an array";\n\t\t\tcase "array":\n\t\t\tcase "tuple":\n\t\t\t\treturn "an object";',
		[
			"data-admission.luau: rule 76 a present declared value of the wrong shape or range makes the load fail with the path, the data untouched",
		],
	),
	m(
		"76",
		"swap",
		A,
		'return node.branches.map(describe).join(" or ");',
		'return node.branches.map(describe).join(" and ");',
		[
			"data-admission.luau: rule 76 a present declared value of the wrong shape or range makes the load fail with the path, the data untouched",
		],
	),
	// JSON for undeclared values.
	m("78", "guard", A, '\t\tif (typeIs(value, "boolean")) return scalar(value, path);\n', "", [
		"data-admission.luau: rule 15 a cached array counts its full visit cost at every use",
	]),
	m(
		"78",
		"guard",
		A,
		'if (typeIs(value, "string")) return scalar(text(value, path), path);',
		'if (typeIs(value, "string")) return scalar(value, path);',
		[
			"data-admission.luau: rule 14 and 76 a write over PAYLOAD_MAX_BYTES throws naming the store and user, a single string over 65536 is refused first",
		],
	),
	m(
		"78",
		"guard",
		A,
		'if (typeIs(value, "number")) return scalar(number(value, path), path);',
		'if (typeIs(value, "number")) return scalar(value, path);',
		[
			"data-unknown-keys.luau: rule 76 and 78 undeclared keys count toward the payload cap, depth and visit limits and must be JSON",
		],
	),
	m(
		"78",
		"guard",
		A,
		'\t\tif (!typeIs(value, "table")) fail(path, `expected JSON, got ${typeOf(value)}`);\n',
		'\t\tif (!typeIs(value, "table")) return value as never;\n',
		[
			"data-unknown-keys.luau: rule 76 and 78 undeclared keys count toward the payload cap, depth and visit limits and must be JSON",
		],
	),
	m(
		"78",
		"swap",
		A,
		"fail(path, `expected JSON, got ${typeOf(value)}`)",
		"fail(`expected JSON, got ${typeOf(value)}`, path)",
		[
			"data-unknown-keys.luau: rule 76 and 78 undeclared keys count toward the payload cap, depth and visit limits and must be JSON",
		],
	),
	m(
		"78",
		"guard",
		A,
		'\t\t\tif (!hasOnlyStringKeys(value)) fail(path, "a table mixing array and object keys");\n',
		"",
		[
			"data-admission.luau: rule 76 a mixed array and object table, and a hole in an undeclared array, are refused at their path",
		],
	),
	m(
		"78",
		"swap",
		A,
		'fail(path, "a table mixing array and object keys")',
		'fail("a table mixing array and object keys", path)',
		[
			"data-admission.luau: rule 76 a mixed array and object table, and a hole in an undeclared array, are refused at their path",
		],
	),
	m("78", "pair", A, "\t\t\t\tif (!rawequal(child, items[i])) same = false;\n", "", [ADM, UNK]),
	m("78", "pair", A, "\t\t\t\tif (!rawequal(child, entry)) same = false;\n", "", [
		"data-admission.luau: rule 15 a frozen table that holds an unfrozen one is copied and frozen throughout",
	]),
	m("78", "swap", A, "copy[i + 1] = child;", "copy[i] = child;", [
		"data-admission.luau: rule 15 a cached array counts its full visit cost at every use",
	]),
	m("78", "swap", A, "json(items[i], `${path}[${i}]`, depth + 1)", "json(items[i], `${path}[${i + 1}]`, depth + 1)", [
		"data-admission.luau: rule 76 depth 28 loads and depth 29 does not, in objects and in arrays",
	]),
	m(
		"78",
		"swap",
		A,
		"const child = json(entry, `${path}.${key}`, depth + 1);",
		"const child = json(entry, `${path}.${key}`, depth);",
		["data-admission.luau: rule 15 a frozen subtree reused where its deepest table passes depth 28 is refused"],
	),
	m(
		"78",
		"guard",
		A,
		"\t\t\t\ttop = math.max(top, height);\n\t\t\t\tif (!rawequal(child, entry)) same = false;",
		"\t\t\t\tif (!rawequal(child, entry)) same = false;",
		["data-admission.luau: rule 15 a frozen subtree reused where its deepest table passes depth 28 is refused"],
	),
	// Table shape tests.
	m("76", "pair", A, "\t\tcount += 1;\n", "", [
		"data-admission.luau: rule 110 unreadable-record diagnostics carry store, user and path, never a value, and a user who rejoins is logged once",
	]),
	m("76", "guard", A, "return count === length ? length : undefined;", "return length;", [
		"data-admission.luau: rule 76 an undeclared table with an interior hole or a stray key is not read as an array",
	]),
	m(
		"76",
		"guard",
		A,
		'for (const [key] of pairs(value)) if (!typeIs(key, "string")) return false;',
		"for (const [key] of pairs(value)) if (key === undefined) return false;",
		[
			"data-admission.luau: rule 76 a mixed array and object table, and a hole in an undeclared array, are refused at their path",
		],
	),
	m(
		"76",
		"guard",
		A,
		'\t\tif (!typeIs(value, "table")) fail(path, `expected ${expected}, got ${typeOf(value)}`);\n',
		'\t\tif (!typeIs(value, "table")) return value as never;\n',
		[
			"data-admission.luau: rule 76 a present declared value of the wrong shape or range makes the load fail with the path, the data untouched",
		],
	),
	m(
		"76",
		"swap",
		A,
		"fail(path, `expected ${expected}, got ${typeOf(value)}`)",
		"fail(`expected ${expected}, got ${typeOf(value)}`, path)",
		[
			"data-admission.luau: rule 76 a present declared value of the wrong shape or range makes the load fail with the path, the data untouched",
		],
	),
	// admit, sealSchema and admitWrite.
	m(
		"4",
		"guard",
		A,
		"\tif (!result.ok) error(`${schema.name}${result.path}: ${result.reason}`, 0);\n\treturn { ...schema, node, defaults: result.value as object };",
		"\tif (!result.ok) return { ...schema, node, defaults: schema.defaults };\n\treturn { ...schema, node, defaults: result.value as object };",
		[
			"data-record-declarations.luau: rule 4 defaults that violate the descriptor throw naming the store and path, and the name stays free",
		],
	),
	m(
		"4",
		"swap",
		A,
		"\tif (!result.ok) error(`${schema.name}${result.path}: ${result.reason}`, 0);\n\treturn { ...schema, node, defaults: result.value as object };",
		"\tif (!result.ok) error(`${result.path}${schema.name}: ${result.reason}`, 0);\n\treturn { ...schema, node, defaults: result.value as object };",
		[
			"data-record-declarations.luau: rule 4 defaults that violate the descriptor throw naming the store and path, and the name stays free",
		],
	),
	m(
		"4",
		"pair",
		A,
		"return { ...schema, node, defaults: result.value as object };",
		"return { ...schema, node, defaults: schema.defaults };",
		[
			"data-admission.luau: rule 15 every admitted table is deeply frozen: the loaded value, declared and undeclared subtrees, and defaults",
		],
	),
	m(
		"4",
		"guard",
		A,
		"const node = schema.node ?? OPEN;\n\tconst result = admit(node, schema.defaults);",
		"const node = schema.node!;\n\tconst result = admit(node, schema.defaults);",
		["data-admission.luau: rule 15 a cached array counts its full visit cost at every use"],
	),
	m(
		"14",
		"guard",
		A,
		'\tif (!typeIs(value, "table")) error(`${schema.name}: a store\'s value is a record, got ${typeOf(value)}`, 0);\n',
		"",
		[ADM, REC],
	),
	m(
		"14",
		"swap",
		A,
		"const result = admit(schema.node ?? OPEN, value);\n\tif (!result.ok) error(`${schema.name}${result.path}: ${result.reason}`, 0);",
		"const result = admit(schema.node ?? OPEN, value);\n\tif (!result.ok) error(`${schema.name}${result.reason}: ${result.path}`, 0);",
		[
			"data-admission.luau: rule 14 a write whose result has a wrong declared value throws at the call with the store and path, never the value, and changes nothing",
		],
	),
	m(
		"14",
		"swap",
		A,
		"const result = admit(schema.node ?? OPEN, value);\n\tif (!result.ok) error(`${schema.name}${result.path}: ${result.reason}`, 0);",
		"const result = admit(schema.node ?? OPEN, value);\n\tif (!result.ok) error(`${result.path}${schema.name}: ${result.reason}`, 0);",
		[
			"data-admission.luau: rule 14 a write whose result has a wrong declared value throws at the call with the store and path, never the value, and changes nothing",
		],
	),
	m(
		"14",
		"guard",
		A,
		"\tconst result = admit(schema.node ?? OPEN, value);\n\tif (!result.ok) error(`${schema.name}${result.path}: ${result.reason}`, 0);\n\treturn result.value as object;",
		"\tconst result = admit(schema.node ?? OPEN, value);\n\tif (!result.ok) return value as object;\n\treturn result.value as object;",
		[
			"data-admission.luau: rule 14 a write whose result has a wrong declared value throws at the call with the store and path, never the value, and changes nothing",
		],
	),
	m(
		"14",
		"swap",
		A,
		"if (marker === FAILURE) return { ok: false, path, reason };",
		"if (marker === FAILURE) return { ok: false, path: reason, reason: path };",
		[
			"data-admission.luau: rule 14 a write whose result has a wrong declared value throws at the call with the store and path, never the value, and changes nothing",
		],
	),
	// Migrations: rules 74, 75, 77.
	m(
		"74",
		"guard",
		A,
		'\tif (payload === undefined) return { kind: "ok", value: schema.defaults, migrated: false };\n',
		"",
		[
			"data-migrations.luau: rule 18 and 74 a key with no envelope, and one with only inbox entries, take the defaults at version N and run no step",
		],
	),
	m(
		"74",
		"swap",
		A,
		'return { kind: "ok", value: schema.defaults, migrated: false };',
		'return { kind: "ok", value: schema.defaults, migrated: true };',
		[
			"data-migrations.luau: rule 18 and 74 a key with no envelope, and one with only inbox entries, take the defaults at version N and run no step",
		],
	),
	m("74", "guard", A, '\tif (version > schema.version) return { kind: "newer", version };\n', "", [
		"data-migrations.luau: rule 77 and durability sequence an older build meets a newer version",
	]),
	m("74", "guard", A, "if (version > schema.version) return", "if (version >= schema.version) return", [
		"data-migrations.luau: rule 74 a record at version v of N runs exactly migrations v to N-1, in order",
	]),
	m(
		"74",
		"swap",
		A,
		"for (let index = version; index < schema.version; index++) {",
		"for (let index = schema.version; index < version; index++) {",
		[
			"data-migrations.luau: rule 110 a failed migration is diagnosed once for a user who rejoins, with the store, user and step",
		],
	),
	m(
		"74",
		"guard",
		A,
		"for (let index = version; index < schema.version; index++) {",
		"for (let index = version + 1; index < schema.version; index++) {",
		[
			"data-migrations.luau: rule 110 a failed migration is diagnosed once for a user who rejoins, with the store, user and step",
		],
	),
	m(
		"74",
		"guard",
		A,
		"for (let index = version; index < schema.version; index++) {",
		"for (let index = version; index < schema.version - 1; index++) {",
		[
			"data-migrations.luau: rule 110 a failed migration is diagnosed once for a user who rejoins, with the store, user and step",
		],
	),
	m(
		"74",
		"guard",
		A,
		"for (let index = version; index < schema.version; index++) {",
		"for (let index = 0; index < schema.version; index++) {",
		["data-migrations.luau: rule 74 a record at version v of N runs exactly migrations v to N-1, in order"],
	),
	m("74", "pair", A, "\t\tmigrated = true;\n", "", [
		"data-migrations.luau: rule 27 and 74 an ambiguous commit never stores the migrated payload under version 0 or the old one under 2",
	]),
	m("74", "pair", A, "\tlet migrated = false;\n", "\tlet migrated = true;\n", [
		"data-migrations.luau: rule 74 a record at version v of N runs exactly migrations v to N-1, in order",
	]),
	m(
		"74",
		"pair",
		A,
		"const result = admit(schema.node ?? OPEN, current, schema.defaults);",
		"const result = admit(schema.node ?? OPEN, current);",
		[
			"data-admission.luau: rule 76 R5 an absent optional object stays absent and a stored one takes the default at its path only when defaults hold one",
		],
	),
	m("74", "swap", A, "const step = steps[index];", "const step = steps[schema.version - 1 - index];", [
		"data-migrations.luau: rule 110 a failed migration is diagnosed once for a user who rejoins, with the store, user and step",
	]),
	m(
		"75",
		"guard",
		A,
		"call(step, copyJson(current) as object, undefined)",
		"call(step, current as object, undefined)",
		[
			"data-migrations.luau: rule 75 a step gets a private copy and the next step sees the merged state, not its sibling's edits",
		],
	),
	m(
		"75",
		"guard",
		A,
		'if (!typeIs(value, "table")) return value;\n\tconst result: Record<string | number, unknown> = {};\n\tfor (const [key, entry] of pairs(value)) result[key as string] = copyJson(entry);',
		'if (!typeIs(value, "table")) return value;\n\tconst result: Record<string | number, unknown> = {};\n\tfor (const [key, entry] of pairs(value)) result[key as string] = entry;',
		[
			"data-migrations.luau: rule 75 a step gets a private copy and the next step sees the merged state, not its sibling's edits",
		],
	),
	m(
		"75",
		"guard",
		A,
		'\tif (!typeIs(value, "table")) return value;\n\tconst result: Record<string | number, unknown> = {};',
		'\tif (!typeIs(value, "table")) return {};\n\tconst result: Record<string | number, unknown> = {};',
		[
			"data-migrations.luau: rule 110 a failed migration is diagnosed once for a user who rejoins, with the store, user and step",
		],
	),
	m("75", "pair", A, "\t\tcurrent = merge(current, patch);\n", "\t\tcurrent = patch;\n", [
		"data-migrations.luau: rule 27 and 74 an ambiguous commit never stores the migrated payload under version 0 or the old one under 2",
	]),
	m("75", "swap", A, "\t\tcurrent = merge(current, patch);\n", "\t\tcurrent = merge(patch, current);\n", [
		"data-migrations.luau: rule 27 and 74 an ambiguous commit never stores the migrated payload under version 0 or the old one under 2",
	]),
	m(
		"75",
		"guard",
		A,
		"if (!isObject(base) || !isObject(patch)) return patch;\n\tconst result: Record<string, unknown> = { ...base };\n\tfor (const [key, entry] of pairs(patch)) result[key] = merge(base[key], entry);",
		"if (!isObject(patch)) return patch;\n\tconst old = base as Record<string, unknown>;\n\tconst result: Record<string, unknown> = { ...old };\n\tfor (const [key, entry] of pairs(patch)) result[key] = merge(old[key], entry);",
		[
			"data-migrations.luau: rule 75 a patch merges objects, replaces everything else verbatim and keeps omitted keys",
		],
	),
	m(
		"75",
		"guard",
		A,
		"if (!isObject(base) || !isObject(patch)) return patch;\n\tconst result: Record<string, unknown> = { ...base };\n\tfor (const [key, entry] of pairs(patch)) result[key] = merge(base[key], entry);",
		"if (!isObject(base)) return patch;\n\tconst added = patch as Record<string, unknown>;\n\tconst result: Record<string, unknown> = { ...base };\n\tfor (const [key, entry] of pairs(added)) result[key] = merge(base[key], entry);",
		[
			"data-migrations.luau: rule 75 a patch merges objects, replaces everything else verbatim and keeps omitted keys",
		],
	),
	m(
		"75",
		"swap",
		A,
		"for (const [key, entry] of pairs(patch)) result[key] = merge(base[key], entry);",
		"for (const [key, entry] of pairs(patch)) result[key] = merge(entry, base[key]);",
		[
			"data-migrations.luau: rule 27 and 74 an ambiguous commit never stores the migrated payload under version 0 or the old one under 2",
		],
	),
	m(
		"75",
		"guard",
		A,
		"for (const [key, entry] of pairs(patch)) result[key] = merge(base[key], entry);",
		"for (const [key, entry] of pairs(patch)) result[key] = entry;",
		[
			"data-migrations.luau: rule 75 a patch merges objects, replaces everything else verbatim and keeps omitted keys",
		],
	),
	m(
		"75",
		"pair",
		A,
		"const result: Record<string, unknown> = { ...base };",
		"const result: Record<string, unknown> = {};",
		[
			"data-migrations.luau: rule 27 and 74 an ambiguous commit never stores the migrated payload under version 0 or the old one under 2",
		],
	),
	m(
		"75",
		"guard",
		A,
		'return typeIs(value, "table") && hasOnlyStringKeys(value);',
		'return typeIs(value, "table");',
		[
			"data-migrations.luau: rule 75 a patch merges objects, replaces everything else verbatim and keeps omitted keys",
		],
	),
	m("75", "guard", A, "if (status === CallResult.Threw) return", "if (false as boolean) return", [
		"data-migrations.luau: rule 110 a failed migration and a failed admission are diagnosed under separate codes",
	]),
	m("75", "guard", A, "if (status === CallResult.Yielded) return", "if (false as boolean) return", [
		"data-migrations.luau: rule 75 a step that throws, yields or returns a non-record fails the load at once and leaves the data untouched",
	]),
	m("75", "guard", A, 'if (!typeIs(patch, "table") || !isObject(patch))', 'if (!typeIs(patch, "table"))', [MIG]),
	m(
		"75",
		"swap",
		A,
		'if (status === CallResult.Threw) return { kind: "unreadable", code: "migration", reason: `${label} threw` };\n\t\tif (status === CallResult.Yielded) return { kind: "unreadable", code: "migration", reason: `${label} yielded` };',
		'if (status === CallResult.Threw) return { kind: "unreadable", code: "migration", reason: `${label} yielded` };\n\t\tif (status === CallResult.Yielded) return { kind: "unreadable", code: "migration", reason: `${label} threw` };',
		["data-migrations.luau: rule 110 a failed migration and a failed admission are diagnosed under separate codes"],
	),
	m(
		"75",
		"swap",
		A,
		"const label = `${schema.name}: migrations[${index}]`;",
		"const label = `${schema.name}: migrations[${index + 1}]`;",
		[
			"data-migrations.luau: rule 110 a failed migration is diagnosed once for a user who rejoins, with the store, user and step",
		],
	),
	m(
		"75",
		"swap",
		A,
		"reason: `${label} returned ${typeOf(patch)}, not a record`,",
		"reason: `${label} returned ${typeOf(current)}, not a record`,",
		[
			"data-migrations.luau: rule 75 a step that throws, yields or returns a non-record fails the load at once and leaves the data untouched",
		],
	),
	m(
		"75",
		"swap",
		A,
		'return { kind: "unreadable", code: "migration", reason: `${label} threw` };',
		'return { kind: "unreadable", code: "admission", reason: `${label} threw` };',
		["data-migrations.luau: rule 110 a failed migration and a failed admission are diagnosed under separate codes"],
	),
	m(
		"75",
		"swap",
		A,
		'return { kind: "unreadable", code: "migration", reason: `${label} yielded` };',
		'return { kind: "unreadable", code: "admission", reason: `${label} yielded` };',
		[MIG],
	),
	m(
		"75",
		"swap",
		A,
		'code: "migration",\n\t\t\t\t\treason: `${label} returned',
		'code: "admission",\n\t\t\t\t\treason: `${label} returned',
		[MIG],
	),
	m(
		"76",
		"swap",
		A,
		'reason: `${schema.name}${result.path}: ${result.reason}${migrated ? ` (after migrating from version ${version})` : ""}`,',
		'reason: `${schema.name}${result.reason}: ${result.path}${migrated ? ` (after migrating from version ${version})` : ""}`,',
		["data-admission.luau: rule 76 100000 values load and 100001 do not, counting the root and every table"],
	),
	m(
		"76",
		"swap",
		A,
		'(after migrating from version ${version})` : ""}`,',
		'(after migrating from version ${schema.version})` : ""}`,',
		["data-migrations.luau: rule 110 a failed migration and a failed admission are diagnosed under separate codes"],
	),
	m("76", "guard", A, '${migrated ? ` (after migrating from version ${version})` : ""}', "", [
		"data-migrations.luau: rule 110 a failed migration and a failed admission are diagnosed under separate codes",
	]),
	m(
		"76",
		"swap",
		A,
		'code: "admission",\n\t\t\t\treason: `${schema.name}${result.path}',
		'code: "migration",\n\t\t\t\treason: `${schema.name}${result.path}',
		["data-migrations.luau: rule 110 a failed migration and a failed admission are diagnosed under separate codes"],
	),
	m(
		"74",
		"guard",
		A,
		'return { kind: "ok", value: result.value as object, migrated };',
		'return { kind: "ok", value: result.value as object, migrated: false };',
		[
			"data-migrations.luau: rule 27 and 74 an ambiguous commit never stores the migrated payload under version 0 or the old one under 2",
		],
	),
	// Generic store: rules 39 and 86 to 90, the transform.
	m(
		"87",
		"guard",
		RC,
		'\t\tif (parsed.kind === "unreadable") {\n\t\t\trecord(parsed);\n\t\t\treturn undefined;\n\t\t}\n',
		"",
		[
			"data-records-limits.luau: rule 110 unreadable and newer records are diagnosed once per store and code with the key and no value",
		],
	),
	m(
		"87",
		"swap",
		RC,
		'const parsed = parseEnvelope(current?.value, "records");',
		'const parsed = parseEnvelope(current?.value, "profiles");',
		["data-records-limits.luau: rule 110 an envelope over the cap is diagnosed once per store without a value"],
	),
	m(
		"87",
		"swap",
		RC,
		'parsed.kind === "ok" ? parsed.envelope : emptyEnvelope("records")',
		'parsed.kind === "ok" ? parsed.envelope : emptyEnvelope("profiles")',
		[
			"data-records-limits.luau: rule 52 a CommandLog flush through the real store dispatches in the open phase and a call after the cutoff is closing",
		],
	),
	m(
		"87",
		"guard",
		RC,
		"\t\tif (envelope.payload !== undefined && envelope.schemaVersion > schema.version) {",
		"\t\tif (envelope.schemaVersion > schema.version) {",
		["data-records.luau: rule 86 and 87 an envelope with no payload is never newer, whatever version it carries"],
	),
	m(
		"87",
		"guard",
		RC,
		"envelope.payload !== undefined && envelope.schemaVersion > schema.version",
		"envelope.payload !== undefined && envelope.schemaVersion >= schema.version",
		["data-records-limits.luau: rule 110 an envelope over the cap is diagnosed once per store without a value"],
	),
	m(
		"87",
		"guard",
		RC,
		'\t\tif (envelope.payload !== undefined && envelope.schemaVersion > schema.version) {\n\t\t\trecord({ kind: "newer", version: envelope.schemaVersion });\n\t\t\treturn undefined;\n\t\t}\n',
		"",
		[
			"data-records.luau: rule 87 a record at a newer version cancels the update, returns unavailable and is diagnosed once",
		],
	),
	m(
		"88",
		"guard",
		RC,
		'\t\tif (envelope.operations.includes(id)) {\n\t\t\trecord({ kind: "replay", envelope });\n\t\t\treturn undefined;\n\t\t}\n',
		"",
		[
			"data-records.luau: rule 87 a result rawequal to the current record stores no payload and records the id, whether passed or made",
		],
	),
	m(
		"88",
		"swap",
		RC,
		'record({ kind: "replay", envelope });',
		'record({ kind: "replay", envelope: emptyEnvelope("records") });',
		[
			"data-records.luau: rule 88 a changing update with a default id retried after an ambiguous commit replays the stored record",
		],
	),
	m(
		"88",
		"guard",
		RC,
		'\t\tif (created <= envelope.operationFloor) {\n\t\t\trecord({ kind: "expired" });\n\t\t\treturn undefined;\n\t\t}\n',
		"",
		[
			"data-records.luau: rule 88 the ring keeps the last 256 ids in order and eviction raises the floor to the evicted id's creation time",
		],
	),
	m("88", "guard", RC, "created <= envelope.operationFloor", "created < envelope.operationFloor", [
		"data-records.luau: rule 88 the ring keeps the last 256 ids in order and eviction raises the floor to the evicted id's creation time",
	]),
	m("88", "swap", RC, "created <= envelope.operationFloor", "envelope.operationFloor <= created", [
		"data-records.luau: rule 13 a generic update or a profile write inside change throws and does not run",
	]),
	m(
		"88",
		"guard",
		RC,
		"operations.size() >= OPERATION_MARKERS_PER_KEY",
		"operations.size() > OPERATION_MARKERS_PER_KEY",
		[
			"data-records.luau: rule 88 a full ring whose oldest id is younger than 60 seconds refuses a new id and evicts once it is older",
		],
	),
	m(
		"88",
		"guard",
		RC,
		'\t\t\tif (!evictable(oldest, input.wall())) {\n\t\t\t\trecord({ kind: "overloaded" });\n\t\t\t\treturn undefined;\n\t\t\t}\n',
		"",
		[
			"data-records.luau: rule 88 a full ring whose oldest id is younger than 60 seconds refuses a new id and evicts once it is older",
		],
	),
	m("88", "guard", RC, "if (!evictable(oldest, input.wall())) {", "if (evictable(oldest, input.wall())) {", [
		"data-records.luau: rule 88 a full ring whose oldest id is younger than 60 seconds refuses a new id and evicts once it is older",
	]),
	m(
		"88",
		"swap",
		RC,
		"operations = operations.filter((_, index) => index > 0);",
		"operations = operations.filter((_, index) => index < operations.size() - 1);",
		[
			"data-records.luau: rule 88 a full ring whose oldest id is younger than 60 seconds refuses a new id and evicts once it is older",
		],
	),
	m("88", "pair", RC, "\t\t\toperations = operations.filter((_, index) => index > 0);\n", "", [
		"data-records.luau: rule 88 a full ring whose oldest id is younger than 60 seconds refuses a new id and evicts once it is older",
	]),
	m("88", "pair", RC, "\t\t\tfloor = math.max(floor, createdAt(oldest) ?? floor);\n", "", [
		"data-records.luau: rule 88 a full ring whose oldest id is younger than 60 seconds refuses a new id and evicts once it is older",
	]),
	m(
		"88",
		"swap",
		RC,
		"floor = math.max(floor, createdAt(oldest) ?? floor);",
		"floor = math.min(floor, createdAt(oldest) ?? floor);",
		[
			"data-records.luau: rule 88 a full ring whose oldest id is younger than 60 seconds refuses a new id and evicts once it is older",
		],
	),
	m(
		"88",
		"swap",
		RC,
		"floor = math.max(floor, createdAt(oldest) ?? floor);",
		"floor = math.max(floor, createdAt(id) ?? floor);",
		[
			"data-records.luau: rule 88 a full ring whose oldest id is younger than 60 seconds refuses a new id and evicts once it is older",
		],
	),
	m("88", "swap", RC, "const oldest = operations[0];", "const oldest = operations[operations.size() - 1];", [
		"data-records.luau: rule 88 a full ring whose oldest id is younger than 60 seconds refuses a new id and evicts once it is older",
	]),
	m("88", "swap", RC, "operations: [...operations, id],", "operations: [id, ...operations],", [
		"data-records.luau: rule 87 a result rawequal to the current record stores no payload and records the id, whether passed or made",
	]),
	m("88", "pair", RC, "operations: [...operations, id],", "operations,", [
		"data-records.luau: rule 87 a missing key starts from frozen defaults and stores the result with the schema version and the id in one write",
	]),
	m("88", "pair", RC, "operationFloor: floor,", "operationFloor: envelope.operationFloor,", [
		"data-records.luau: rule 88 a full ring whose oldest id is younger than 60 seconds refuses a new id and evicts once it is older",
	]),
	m("86", "pair", RC, "\t\t...envelope,\n", '\t\t...emptyEnvelope("records"),\n', [
		"data-records-limits.luau: rule 110 an envelope over the cap is diagnosed once per store without a value",
	]),
	m(
		"86",
		"guard",
		RC,
		'record({ kind: "unreadable", reason: loaded.kind === "unreadable" ? loaded.reason : "newer" });',
		'record({ kind: "unreadable", reason: "x" });',
		[
			"data-records.luau: rule 110 an update and a read of a record that does not load carry the same exact diagnostic",
		],
	),
	m("87", "guard", RC, "\t\t\tif (rawequal(changed, loaded.value)) return loaded.value;\n", "", [
		"data-records-limits.luau: rule 87 a no-op update of a stored record over 1,000,000 bytes stores no payload and records the id",
	]),
	m(
		"87",
		"guard",
		RC,
		"const changed = dataCallback(input.change, loaded.value, undefined);",
		"const changed = input.change(loaded.value);",
		[REC, LIM],
	),
	m("87", "guard", RC, "\t\t\tconst admitted = admitWrite(schema, changed);\n", "\t\t\tconst admitted = changed;\n", [
		"data-records-limits.luau: rule 39 a string over 65,536 bytes is refused by admission at the call",
	]),
	m(
		"87",
		"guard",
		RC,
		"\t\t\tif (bytes > RECORD_PAYLOAD_MAX_BYTES)\n\t\t\t\terror(`${schema.name}: ${input.key}'s record is ${bytes} bytes, over ${RECORD_PAYLOAD_MAX_BYTES}`, 0);\n",
		"",
		[
			"data-records-limits.luau: rule 87 a record over 1,000,000 bytes throws at the call naming the store and key, stores nothing and records no id",
		],
	),
	m("87", "guard", RC, "if (bytes > RECORD_PAYLOAD_MAX_BYTES)", "if (bytes >= RECORD_PAYLOAD_MAX_BYTES)", [
		"data-records-limits.luau: rule 87 a record of exactly 1,000,000 bytes with a full ring of 64 byte ids is written within the record sections sum",
	]),
	m(
		"87",
		"swap",
		RC,
		"is ${bytes} bytes, over ${RECORD_PAYLOAD_MAX_BYTES}`",
		"is ${RECORD_PAYLOAD_MAX_BYTES} bytes, over ${bytes}`",
		[
			"data-records-limits.luau: rule 87 a record over 1,000,000 bytes throws at the call naming the store and key, stores nothing and records no id",
		],
	),
	m(
		"87",
		"swap",
		RC,
		"error(`${schema.name}: ${input.key}'s record is",
		"error(`${input.key}: ${schema.name}'s record is",
		[
			"data-records-limits.luau: rule 87 a record over 1,000,000 bytes throws at the call naming the store and key, stores nothing and records no id",
		],
	),
	m(
		"87",
		"guard",
		RC,
		'\t\tif (!ok) {\n\t\t\trecord({ kind: "refused", error: result });\n\t\t\treturn undefined;\n\t\t}\n',
		"",
		["data-records.luau: rule 13 a generic update or a profile write inside change throws and does not run"],
	),
	m(
		"87",
		"pair",
		RC,
		"\t\tconst unchanged = rawequal(value, loaded.value);\n\t\tconst keep = unchanged && !loaded.migrated;",
		"\t\tconst keep = rawequal(value, loaded.value);",
		[
			"data-records.luau: rule 87 an older stored version writes the migrated value even when change returns it unchanged",
		],
	),
	m("87", "guard", RC, "const keep = unchanged && !loaded.migrated;", "const keep = unchanged;", [
		"data-records.luau: rule 87 an older stored version writes the migrated value even when change returns it unchanged",
	]),
	m("87", "guard", RC, "const keep = unchanged && !loaded.migrated;", "const keep = !loaded.migrated;", [
		"data-records.luau: rule 87 a missing key starts from frozen defaults and stores the result with the schema version and the id in one write",
	]),
	m("87", "swap", RC, "payload: keep ? envelope.payload : value,", "payload: keep ? value : envelope.payload,", [
		"data-records.luau: rule 87 a missing key starts from frozen defaults and stores the result with the schema version and the id in one write",
	]),
	m("87", "pair", RC, "payload: keep ? envelope.payload : value,", "payload: envelope.payload,", [
		"data-records.luau: rule 87 a missing key starts from frozen defaults and stores the result with the schema version and the id in one write",
	]),
	m("87", "pair", RC, "payload: keep ? envelope.payload : value,", "payload: value,", [
		"data-records.luau: rule 87 on an absent key a no-op records the id in an envelope with no payload that reads undefined",
	]),
	m(
		"87",
		"swap",
		RC,
		"schemaVersion: keep ? envelope.schemaVersion : schema.version,",
		"schemaVersion: keep ? schema.version : envelope.schemaVersion,",
		[
			"data-records.luau: rule 87 a missing key starts from frozen defaults and stores the result with the schema version and the id in one write",
		],
	),
	m(
		"87",
		"pair",
		RC,
		"schemaVersion: keep ? envelope.schemaVersion : schema.version,",
		"schemaVersion: envelope.schemaVersion,",
		[
			"data-records.luau: rule 87 a missing key starts from frozen defaults and stores the result with the schema version and the id in one write",
		],
	),
	m(
		"87",
		"pair",
		RC,
		"schemaVersion: keep ? envelope.schemaVersion : schema.version,",
		"schemaVersion: schema.version,",
		[
			"data-records.luau: rule 87 on an absent key a no-op records the id in an envelope with no payload that reads undefined",
		],
	),
	m(
		"39",
		"guard",
		RC,
		'\t\tif (bytes > MAX_ENVELOPE_BYTES) {\n\t\t\trecord({ kind: "too-large", bytes });\n\t\t\treturn undefined;\n\t\t}\n',
		"",
		["data-records-limits.luau: rule 110 an envelope over the cap is diagnosed once per store without a value"],
	),
	m("39", "guard", RC, "if (bytes > MAX_ENVELOPE_BYTES) {", "if (bytes >= MAX_ENVELOPE_BYTES) {", [
		"data-records-limits.luau: rule 39 other sections push the final envelope over the cap: the update cancels unavailable, and the cap itself is exact",
	]),
	m("39", "swap", RC, 'record({ kind: "too-large", bytes });', 'record({ kind: "too-large", bytes: bytes - 1 });', [
		"data-records-limits.luau: rule 39 other sections push the final envelope over the cap: the update cancels unavailable, and the cap itself is exact",
	]),
	m("39", "pair", RC, "const bytes = envelopeBytes(written);", "const bytes = jsonBytes(value);", [
		"data-records-limits.luau: rule 110 an envelope over the cap is diagnosed once per store without a value",
	]),
	m("87", "pair", RC, '\t\trecord({ kind: "written", value, bytes });\n', "", [
		"data-records-limits.luau: rule 39 other sections push the final envelope over the cap: the update cancels unavailable, and the cap itself is exact",
	]),
	m(
		"87",
		"swap",
		RC,
		'record({ kind: "written", value, bytes });',
		'record({ kind: "written", value, bytes: bytes + 8192 });',
		["data-records-limits.luau: rule 36 a first update is weighed at 1,024 bytes and the bound is exact"],
	),
	m("87", "pair", RC, "userIds: current?.userIds ?? [],", "userIds: [],", [
		"data-records.luau: rule 87 an update keeps the key's user ids and metadata",
	]),
	m("87", "pair", RC, "metadata: current?.metadata ?? {} };", "metadata: {} };", [
		"data-records.luau: rule 87 an update keeps the key's user ids and metadata",
	]),
	// Generic store: the calls.
	m(
		"86",
		"guard",
		RC,
		'if (!typeIs(key, "string") || key.size() < 1 || key.size() > KEY_MAX_BYTES)\n\t\t\terror(`${this.schema.name}: a key is 1 to ${KEY_MAX_BYTES} bytes`, 3);\n\t\treturn key;',
		"const text = key as string;\n\t\tif (text.size() < 1 || text.size() > KEY_MAX_BYTES)\n\t\t\terror(`${this.schema.name}: a key is 1 to ${KEY_MAX_BYTES} bytes`, 3);\n\t\treturn text;",
		[
			"data-records.luau: rule 86 a key is 1 to 50 bytes: 0 and 51 throw at the call naming the store and send nothing, 50 works",
		],
	),
	m(
		"86",
		"guard",
		RC,
		'!typeIs(key, "string") || key.size() < 1 || key.size() > KEY_MAX_BYTES',
		'!typeIs(key, "string") || key.size() > KEY_MAX_BYTES',
		[
			"data-records.luau: rule 86 a key is 1 to 50 bytes: 0 and 51 throw at the call naming the store and send nothing, 50 works",
		],
	),
	m(
		"86",
		"guard",
		RC,
		'!typeIs(key, "string") || key.size() < 1 || key.size() > KEY_MAX_BYTES',
		'!typeIs(key, "string") || key.size() < 1',
		[
			"data-records.luau: rule 86 a key is 1 to 50 bytes: 0 and 51 throw at the call naming the store and send nothing, 50 works",
		],
	),
	m("86", "swap", RC, "key.size() > KEY_MAX_BYTES", "key.size() >= KEY_MAX_BYTES", [
		"data-records.luau: rule 86 a key is 1 to 50 bytes: 0 and 51 throw at the call naming the store and send nothing, 50 works",
	]),
	m("86", "swap", RC, "key.size() > KEY_MAX_BYTES", "KEY_MAX_BYTES > key.size()", [
		"data-records.luau: rule 13 a generic update or a profile write inside change throws and does not run",
	]),
	m("86", "swap", RC, "key.size() < 1", "1 < key.size()", [
		"data-records.luau: rule 13 a generic update or a profile write inside change throws and does not run",
	]),
	m(
		"86",
		"swap",
		RC,
		"error(`${this.schema.name}: a key is 1 to ${KEY_MAX_BYTES} bytes`, 3)",
		"error(`${key}: a key is 1 to ${KEY_MAX_BYTES} bytes`, 3)",
		[
			"data-records.luau: rule 86 a key is 1 to 50 bytes: 0 and 51 throw at the call naming the store and send nothing, 50 works",
		],
	),
	m(
		"86",
		"guard",
		RC,
		'\t\tthis.check(key);\n\t\tconst deadline = bootClock() + CALL_DEADLINE_SECONDS;\n\t\tconst ticket = this.engine.scheduler.submit({\n\t\t\tstore: this.schema.name,\n\t\t\tkey,\n\t\t\tpriority: Priority.Ordinary,\n\t\t\tkind: "read",',
		'\t\tconst deadline = bootClock() + CALL_DEADLINE_SECONDS;\n\t\tconst ticket = this.engine.scheduler.submit({\n\t\t\tstore: this.schema.name,\n\t\t\tkey,\n\t\t\tpriority: Priority.Ordinary,\n\t\t\tkind: "read",',
		[
			"data-records.luau: rule 86 a key is 1 to 50 bytes: 0 and 51 throw at the call naming the store and send nothing, 50 works",
		],
	),
	m("87", "guard", RC, "\t\tassertOutsideCallback();\n\t\tthis.check(key);\n", "\t\tthis.check(key);\n", [
		"data-records.luau: rule 13 a generic update inside a profile data callback throws and the write is cancelled",
	]),
	m("87", "guard", RC, "\t\tassertOutsideCallback();\n\t\tthis.check(key);\n", "\t\tassertOutsideCallback();\n", [
		"data-records.luau: rule 86 a key is 1 to 50 bytes: 0 and 51 throw at the call naming the store and send nothing, 50 works",
	]),
	m(
		"86",
		"swap",
		RC,
		'priority: Priority.Ordinary,\n\t\t\tkind: "read",',
		'priority: Priority.Load,\n\t\t\tkind: "read",',
		[
			"data-records-limits.luau: rule 32 generic reads and updates are ordinary: they wait while the budget is at the reserve",
		],
	),
	m(
		"87",
		"swap",
		RC,
		'priority: Priority.Ordinary,\n\t\t\tkind: "update",',
		'priority: Priority.Load,\n\t\t\tkind: "update",',
		[
			"data-records-limits.luau: rule 32 generic reads and updates are ordinary: they wait while the budget is at the reserve",
		],
	),
	m(
		"86",
		"swap",
		RC,
		'kind: "read",\n\t\t\tdispatchDeadline: deadline,\n\t\t\tbytes: this.weigh(key),',
		'kind: "update",\n\t\t\tdispatchDeadline: deadline,\n\t\t\tbytes: this.weigh(key),',
		[
			"data-records-limits.luau: rule 110 unreadable and newer records are diagnosed once per store and code with the key and no value",
		],
	),
	m(
		"86",
		"swap",
		RC,
		'const settled = ticket.wait(deadline) ?? { kind: ticket.dispatched ? "uncertain" : "timeout" };\n\t\tif (settled.kind !== "done") return { kind: settled.kind };\n\t\tif (settled.outcome.kind === "error") return',
		'const settled = ticket.wait(deadline) ?? { kind: ticket.dispatched ? "timeout" : "uncertain" };\n\t\tif (settled.kind !== "done") return { kind: settled.kind };\n\t\tif (settled.outcome.kind === "error") return',
		["data-records-limits.luau: rule 90 a dispatched request with no answer by the deadline settles uncertain"],
	),
	m(
		"87",
		"swap",
		RC,
		'const settled = ticket.wait(deadline) ?? { kind: ticket.dispatched ? "uncertain" : "timeout" };\n\t\tif (settled.kind !== "done") return { kind: settled.kind };\n\t\tif (settled.outcome.kind === "error" || decision',
		'const settled = ticket.wait(deadline) ?? { kind: ticket.dispatched ? "timeout" : "uncertain" };\n\t\tif (settled.kind !== "done") return { kind: settled.kind };\n\t\tif (settled.outcome.kind === "error" || decision',
		["data-records-limits.luau: rule 90 a dispatched request with no answer by the deadline settles uncertain"],
	),
	m(
		"90",
		"guard",
		RC,
		'\t\tif (settled.kind !== "done") return { kind: settled.kind };\n\t\tif (settled.outcome.kind === "error") return { kind: "unavailable" };',
		'\t\tif (settled.kind !== "done") return { kind: "ok", value: undefined };\n\t\tif (settled.outcome.kind === "error") return { kind: "unavailable" };',
		["data-records-limits.luau: rule 90 a dispatched request with no answer by the deadline settles uncertain"],
	),
	m(
		"90",
		"guard",
		RC,
		'\t\tif (settled.outcome.kind === "error") return { kind: "unavailable" };\n\t\tconst parsed',
		'\t\tif (settled.outcome.kind === "error") return { kind: "ok", value: undefined };\n\t\tconst parsed',
		[
			"data-records-limits.luau: rule 90 a non-retryable storage error is unavailable and diagnosed once per store and code",
		],
	),
	m(
		"90",
		"guard",
		RC,
		'if (settled.outcome.kind === "error" || decision === undefined) return { kind: "unavailable" };',
		'if (decision === undefined) return { kind: "unavailable" };',
		[
			"data-records.luau: rule 90 an attempt after an ambiguous commit that fails for good is unavailable, not the first attempt's result",
		],
	),
	m(
		"90",
		"swap",
		RC,
		'if (settled.kind !== "done") return { kind: settled.kind };\n\t\tif (settled.outcome.kind === "error" || decision',
		'if (settled.kind !== "done") return { kind: "timeout" };\n\t\tif (settled.outcome.kind === "error" || decision',
		[
			"data-records-limits.luau: rule 52 a CommandLog flush through the real store dispatches in the open phase and a call after the cutoff is closing",
		],
	),
	m(
		"86",
		"swap",
		RC,
		'const parsed = parseEnvelope(settled.outcome.value?.value, "records");',
		'const parsed = parseEnvelope(settled.outcome.value?.value, "profiles");',
		[
			"data-records.luau: rule 86 a key is 1 to 50 bytes: 0 and 51 throw at the call naming the store and send nothing, 50 works",
		],
	),
	m(
		"86",
		"guard",
		RC,
		'if (parsed.kind === "absent") return { kind: "ok", value: undefined };',
		'if (parsed.kind === "absent") return { kind: "unavailable" };',
		["data-records.luau: rule 86 an absent key and an envelope with no payload read undefined, never defaults"],
	),
	m(
		"86",
		"guard",
		RC,
		'if (parsed.kind === "unreadable") return this.unavailable(key, "unreadable", "is unreadable");',
		'if (parsed.kind === "unreadable") return { kind: "ok", value: undefined };',
		["data-records.luau: rule 86 an unreadable record and a newer record are unavailable and nothing is written"],
	),
	m(
		"86",
		"guard",
		RC,
		'\t\tif (parsed.envelope.payload === undefined) return { kind: "ok", value: undefined };\n',
		"",
		["data-records.luau: rule 86 an absent key and an envelope with no payload read undefined, never defaults"],
	),
	m("86", "pair", RC, "\t\tthis.remember(key, envelopeBytes(parsed.envelope));\n", "", [
		"data-records-limits.luau: rule 36 reads of a key with a large record are paced by that record's size",
	]),
	m(
		"86",
		"guard",
		RC,
		'if (loaded.kind === "newer") return this.unavailable(key, "newer", "was written by a newer build");',
		'if (loaded.kind === "newer") return { kind: "ok", value: undefined };',
		["data-records.luau: rule 86 an unreadable record and a newer record are unavailable and nothing is written"],
	),
	m(
		"86",
		"guard",
		RC,
		'if (loaded.kind === "unreadable") return this.unavailable(key, "unreadable", `did not load: ${loaded.reason}`);',
		'if (loaded.kind === "unreadable") return { kind: "ok", value: undefined };',
		["data-records.luau: rule 86 an unreadable record and a newer record are unavailable and nothing is written"],
	),
	m(
		"86",
		"swap",
		RC,
		'return this.unavailable(key, "newer", "was written by a newer build");\n\t\tif (loaded.kind === "unreadable")',
		'return this.unavailable(key, "unreadable", "was written by a newer build");\n\t\tif (loaded.kind === "unreadable")',
		[
			"data-records-limits.luau: rule 110 unreadable and newer records are diagnosed once per store and code with the key and no value",
		],
	),
	m(
		"86",
		"swap",
		RC,
		'return this.unavailable(key, "unreadable", `did not load: ${loaded.reason}`);\n\t\treturn { kind: "ok", value: loaded.value };',
		'return this.unavailable(key, "newer", `did not load: ${loaded.reason}`);\n\t\treturn { kind: "ok", value: loaded.value };',
		[
			"data-records-limits.luau: rule 110 unreadable and newer records are diagnosed once per store and code with the key and no value",
		],
	),
	m(
		"86",
		"swap",
		RC,
		'this.unavailable(key, "unreadable", "is unreadable")',
		'this.unavailable(key, "newer", "is unreadable")',
		[REC, LIM],
	),
	m(
		"86",
		"swap",
		RC,
		'return this.unavailable(key, "newer", "was written by a newer build");\n\t\t\tcase "unreadable":',
		'return this.unavailable(key, "unreadable", "was written by a newer build");\n\t\t\tcase "unreadable":',
		[
			"data-records.luau: rule 87 a record at a newer version cancels the update, returns unavailable and is diagnosed once",
		],
	),
	m(
		"86",
		"swap",
		RC,
		'return this.unavailable(key, "unreadable", `did not load: ${decision.reason}`);',
		'return this.unavailable(key, "newer", `did not load: ${decision.reason}`);',
		[
			"data-records-limits.luau: rule 110 unreadable and newer records are diagnosed once per store and code with the key and no value",
		],
	),
	m(
		"39",
		"swap",
		RC,
		'return this.unavailable(key, "large", `would make the envelope ${decision.bytes} bytes`);',
		'return this.unavailable(key, "unreadable", `would make the envelope ${decision.bytes} bytes`);',
		[LIM],
	),
	m(
		"110",
		"swap",
		RC,
		"this.engine.diagnose(`record:${this.schema.name}:${code}`, `${this.schema.name}: ${key} ${reason}`);",
		"this.engine.diagnose(`record:${this.schema.name}:${code}`, `${key}: ${this.schema.name} ${reason}`);",
		["data-records-limits.luau: rule 110 an envelope over the cap is diagnosed once per store without a value"],
	),
	m(
		"110",
		"guard",
		RC,
		"this.engine.diagnose(`record:${this.schema.name}:${code}`,",
		"this.engine.diagnose(`record:${this.schema.name}:${key}`,",
		["data-records-limits.luau: rule 110 an envelope over the cap is diagnosed once per store without a value"],
	),
	m(
		"110",
		"guard",
		RC,
		"this.engine.diagnose(`record:${this.schema.name}:${code}`,",
		"this.engine.diagnose(`record:${code}`,",
		[REC, LIM],
	),
	m(
		"87",
		"guard",
		RC,
		'\t\t\tcase "refused":\n\t\t\t\treturn error(decision.error, 0);\n',
		'\t\t\tcase "refused":\n\t\t\t\treturn { kind: "unavailable" };\n',
		["data-records.luau: rule 13 a generic update or a profile write inside change throws and does not run"],
	),
	m("87", "pair", RC, "\t\t\t\tthis.remember(key, decision.bytes);\n", "", [
		"data-records-limits.luau: rule 36 a first update is weighed at 1,024 bytes and the bound is exact",
	]),
	m(
		"87",
		"swap",
		RC,
		'return { kind: "ok", value: decision.value, replayed: false };',
		'return { kind: "ok", value: decision.value, replayed: true };',
		[
			"data-records.luau: rule 87 a missing key starts from frozen defaults and stores the result with the schema version and the id in one write",
		],
	),
	m(
		"88",
		"swap",
		RC,
		'return { kind: "ok", value: loaded.value, replayed: true };',
		'return { kind: "ok", value: loaded.value, replayed: false };',
		[
			"data-records.luau: rule 87 a result rawequal to the current record stores no payload and records the id, whether passed or made",
		],
	),
	m(
		"88",
		"guard",
		RC,
		'if (loaded.kind !== "ok") return { kind: "unavailable" };\n\t\t\t\t\treturn { kind: "ok", value: loaded.value, replayed: true };',
		'if (loaded.kind !== "ok") return { kind: "ok", value: {}, replayed: true };\n\t\t\t\t\treturn { kind: "ok", value: loaded.value, replayed: true };',
		["data-records.luau: rule 88 a replay of an id held by a record that no longer loads is unavailable"],
	),
	m(
		"88",
		"swap",
		RC,
		"const loaded = loadRecord(this.schema, decision.envelope.payload, decision.envelope.schemaVersion);",
		"const loaded = loadRecord(this.schema, decision.envelope.payload, this.schema.version);",
		[
			"data-records.luau: rule 88 a replay of an id held by a record stored at an older version returns the record migrated",
		],
	),
	m(
		"90",
		"swap",
		RC,
		"\t\t\tdefault:\n\t\t\t\treturn { kind: decision.kind };",
		'\t\t\tdefault:\n\t\t\t\treturn { kind: "unavailable" };',
		[
			"data-records.luau: rule 88 a full ring whose oldest id is younger than 60 seconds refuses a new id and evicts once it is older",
		],
	),
	m(
		"88",
		"swap",
		RC,
		'const sent = id ?? newOperationId(port);\n\t\tif (!typeIs(sent, "string") || sent.size() > OPERATION_ID_MAX_BYTES || createdAt(sent) === undefined)',
		"const sent = id ?? newOperationId(port);\n\t\tif (sent.size() > OPERATION_ID_MAX_BYTES || createdAt(sent) === undefined)",
		[
			"data-records.luau: rule 88 an id over 64 bytes or not made by operationId throws at the call naming the store and sends nothing",
		],
	),
	m(
		"88",
		"guard",
		RC,
		"|| sent.size() > OPERATION_ID_MAX_BYTES || createdAt(sent) === undefined)",
		"|| createdAt(sent) === undefined)",
		[
			"data-records.luau: rule 88 an id over 64 bytes or not made by operationId throws at the call naming the store and sends nothing",
		],
	),
	m(
		"88",
		"guard",
		RC,
		"|| sent.size() > OPERATION_ID_MAX_BYTES || createdAt(sent) === undefined)",
		"|| sent.size() > OPERATION_ID_MAX_BYTES)",
		[
			"data-records.luau: rule 88 an id over 64 bytes or not made by operationId throws at the call naming the store and sends nothing",
		],
	),
	m("88", "swap", RC, "sent.size() > OPERATION_ID_MAX_BYTES", "sent.size() >= OPERATION_ID_MAX_BYTES", [
		"data-records.luau: rule 88 an id over 64 bytes or not made by operationId throws at the call naming the store and sends nothing",
	]),
	m("88", "swap", RC, "sent.size() > OPERATION_ID_MAX_BYTES", "OPERATION_ID_MAX_BYTES > sent.size()", [
		"data-records.luau: rule 13 a generic update or a profile write inside change throws and does not run",
	]),
	m(
		"88",
		"guard",
		RC,
		'\t\tif (ageOf(sent, port.wall(), OPERATION_ID_MAX_AGE_SECONDS) !== "fresh") return { kind: "expired" };\n',
		"",
		[
			"data-records.luau: rule 88 an id older than 3,600 seconds is expired before any backend call and the boundary is exact",
		],
	),
	m(
		"88",
		"swap",
		RC,
		"ageOf(sent, port.wall(), OPERATION_ID_MAX_AGE_SECONDS)",
		"ageOf(sent, OPERATION_ID_MAX_AGE_SECONDS, port.wall())",
		["data-records.luau: rule 13 a generic update or a profile write inside change throws and does not run"],
	),
	m(
		"88",
		"swap",
		RC,
		'if (ageOf(sent, port.wall(), OPERATION_ID_MAX_AGE_SECONDS) !== "fresh") return { kind: "expired" };',
		'if (ageOf(sent, port.wall(), OPERATION_ID_MAX_AGE_SECONDS) === "malformed") return { kind: "expired" };',
		[
			"data-records.luau: rule 88 an id older than 3,600 seconds is expired before any backend call and the boundary is exact",
		],
	),
	m(
		"88",
		"guard",
		RC,
		"const input: UpdateInput = { schema: this.schema, key, id: sent, change, wall: port.wall };",
		"const input: UpdateInput = { schema: this.schema, key, id: sent, change, wall: () => 0 };",
		[
			"data-records.luau: rule 88 a full ring whose oldest id is younger than 60 seconds refuses a new id and evicts once it is older",
		],
	),
	m(
		"88",
		"swap",
		RC,
		"const input: UpdateInput = { schema: this.schema, key, id: sent, change, wall: port.wall };",
		"const input: UpdateInput = { schema: this.schema, key, id: key, change, wall: port.wall };",
		["data-records.luau: rule 13 a generic update or a profile write inside change throws and does not run"],
	),
	m(
		"88",
		"swap",
		RC,
		"transform: () => (current) => updateRecord(current, input, (d) => (decision = d)),",
		"transform: () => (current) => updateRecord(current, { ...input, id: newOperationId(port) }, (d) => (decision = d)),",
		[
			"data-records.luau: rule 87 a missing key starts from frozen defaults and stores the result with the schema version and the id in one write",
		],
	),
	m(
		"36",
		"swap",
		RC,
		"return math.max(this.sizes.get(key) ?? 0, 1024);",
		"return math.min(this.sizes.get(key) ?? 0, 1024);",
		["data-records-limits.luau: rule 36 a first update is weighed at 1,024 bytes and the bound is exact"],
	),
	m(
		"36",
		"swap",
		RC,
		"return math.max(this.sizes.get(key) ?? 0, 1024);",
		"return math.max(this.sizes.get(key) ?? 0, 4096);",
		["data-records-limits.luau: rule 36 a first update is weighed at 1,024 bytes and the bound is exact"],
	),
	m(
		"36",
		"guard",
		RC,
		"if (bytes > 4096) this.sizes.set(key, bytes);",
		"if (bytes >= 4096) this.sizes.set(key, bytes);",
		["data-records-limits.luau: rule 36 a record of exactly 4,096 bytes is weighed at the floor"],
	),
	m("36", "pair", RC, "\t\telse this.sizes.delete(key);\n", "", [
		"data-records-limits.luau: rule 36 a record of exactly 4,096 bytes is weighed at the floor",
	]),
	m("36", "pair", RC, "if (bytes > 4096) this.sizes.set(key, bytes);", "if (bytes > 4096) bytes;", [
		"data-records-limits.luau: rule 36 a first update is weighed at 1,024 bytes and the bound is exact",
	]),
	m("36", "swap", RC, "bytes: this.weigh(key),\n\t\t\ttransform", "bytes: 1024,\n\t\t\ttransform", [
		"data-records-limits.luau: rule 36 a first update is weighed at 1,024 bytes and the bound is exact",
	]),
	m(
		"36",
		"swap",
		RC,
		'kind: "read",\n\t\t\tdispatchDeadline: deadline,\n\t\t\tbytes: this.weigh(key),',
		'kind: "read",\n\t\t\tdispatchDeadline: deadline,\n\t\t\tbytes: 1024,',
		["data-records-limits.luau: rule 36 reads of a key with a large record are paced by that record's size"],
	),
	// Declarations and bindings: rules 1 to 4 and 104.
	m(
		"2",
		"guard",
		RD,
		"name.size() === 0 || name.size() > STORE_NAME_MAX_BYTES",
		"name.size() > STORE_NAME_MAX_BYTES",
		[
			"data-record-declarations.luau: rule 3 names are dot-separated letter-led segments and Crucible is reserved as the first",
		],
	),
	m("2", "guard", RD, "name.size() === 0 || name.size() > STORE_NAME_MAX_BYTES", "name.size() === 0", [
		"data-record-declarations.luau: rule 2 a store name is at most 50 bytes",
	]),
	m(
		"2",
		"guard",
		RD,
		"name.size() === 0 || name.size() > STORE_NAME_MAX_BYTES",
		"name.size() === 0 || name.size() >= STORE_NAME_MAX_BYTES",
		["data-record-declarations.luau: rule 2 a store name is at most 50 bytes"],
	),
	m("2", "swap", RD, "name.size() > STORE_NAME_MAX_BYTES", "STORE_NAME_MAX_BYTES > name.size()", [
		"data-record-declarations.luau: rule 104 a call before binding completes normally when binding happens within its deadline",
	]),
	m(
		"2",
		"swap",
		RD,
		"error(`${name}: a store name is 1 to ${STORE_NAME_MAX_BYTES} bytes`, 4);",
		"error(`${STORE_NAME_MAX_BYTES}: a store name is 1 to ${name} bytes`, 4);",
		["data-record-declarations.luau: rule 2 a store name is at most 50 bytes"],
	),
	m(
		"3",
		"guard",
		RD,
		'\t\t\tif (segment.match("^[A-Za-z][A-Za-z0-9_]*$")[0] === undefined)',
		'\t\t\tif (segment.match("^[A-Za-z][A-Za-z0-9_]*")[0] === undefined)',
		[
			"data-record-declarations.luau: rule 3 names are dot-separated letter-led segments and Crucible is reserved as the first",
		],
	),
	m("3", "guard", RD, '"^[A-Za-z][A-Za-z0-9_]*$"', '"[A-Za-z][A-Za-z0-9_]*$"', [
		"data-record-declarations.luau: rule 3 names are dot-separated letter-led segments and Crucible is reserved as the first",
	]),
	m("3", "guard", RD, '"^[A-Za-z][A-Za-z0-9_]*$"', '"^[A-Za-z0-9_][A-Za-z0-9_]*$"', [
		"data-record-declarations.luau: rule 3 names are dot-separated letter-led segments and Crucible is reserved as the first",
	]),
	m("3", "guard", RD, '"^[A-Za-z][A-Za-z0-9_]*$"', '"^[A-Za-z][A-Za-z0-9]*$"', [
		"data-record-declarations.luau: rule 3 names are dot-separated letter-led segments and Crucible is reserved as the first",
	]),
	m("3", "guard", RD, '"^[A-Za-z][A-Za-z0-9_]*$"', '"^[A-Za-z][A-Za-z_]*$"', [
		"data-record-declarations.luau: rule 3 a realm holds at most 256 data declarations",
	]),
	m("3", "guard", RD, '"^[A-Za-z][A-Za-z0-9_]*$"', '"^[a-z][A-Za-z0-9_]*$"', [
		"data-record-declarations.luau: rule 104 a call before binding completes normally when binding happens within its deadline",
	]),
	m("3", "guard", RD, '"^[A-Za-z][A-Za-z0-9_]*$"', '"^[A-Z][A-Za-z0-9_]*$"', [
		"data-record-declarations.luau: rule 3 names are dot-separated letter-led segments and Crucible is reserved as the first",
	]),
	m("3", "guard", RD, '"^[A-Za-z][A-Za-z0-9_]*$"', '"^[A-Za-z][a-z0-9_]*$"', [
		"data-record-declarations.luau: rule 3 names are dot-separated letter-led segments and Crucible is reserved as the first",
	]),
	m("3", "guard", RD, '"^[A-Za-z][A-Za-z0-9_]*$"', '"^[A-Za-z][A-Za-z0-9_]+$"', [
		"data-record-declarations.luau: rule 3 names are dot-separated letter-led segments and Crucible is reserved as the first",
	]),
	m("3", "guard", RD, 'const segments = name.split(".");', 'const segments = name.split("%.");', [
		"data-record-declarations.luau: rule 104 a call before binding completes normally when binding happens within its deadline",
	]),
	m(
		"3",
		"guard",
		RD,
		'\t\tif (segments[0] === "Crucible") error(`${name}: the name Crucible is reserved`, 4);\n',
		"",
		[
			"data-record-declarations.luau: rule 3 names are dot-separated letter-led segments and Crucible is reserved as the first",
		],
	),
	m("3", "swap", RD, 'if (segments[0] === "Crucible")', 'if (segments[segments.size() - 1] === "Crucible")', [
		"data-record-declarations.luau: rule 3 names are dot-separated letter-led segments and Crucible is reserved as the first",
	]),
	m(
		"3",
		"swap",
		RD,
		"error(`${name}: a store name is dot-separated segments of letters, digits and underscores`, 4);",
		"error(`${name}: the name Crucible is reserved`, 4);",
		[
			"data-record-declarations.luau: rule 3 names are dot-separated letter-led segments and Crucible is reserved as the first",
		],
	),
	m(
		"3",
		"guard",
		RD,
		"\t\tif (sealed)\n\t\t\terror(`${name} was declared after the data registry sealed; declare stores in modules that boot loads`, 3);\n",
		"",
		[
			"data-record-declarations.luau: rule 3 a declaration after the registry seals throws and sealing off allows it again",
		],
	),
	m(
		"3",
		"swap",
		RD,
		"export function sealDeclarations(value: boolean) {\n\tsealed = value;",
		"export function sealDeclarations(value: boolean) {\n\tsealed = !value;",
		[
			"data-record-declarations.luau: rule 3 a declaration after the registry seals throws and sealing off allows it again",
		],
	),
	m(
		"3",
		"pair",
		RD,
		"export function sealDeclarations(value: boolean) {\n\tsealed = value;",
		"export function sealDeclarations(value: boolean) {\n\tsealed = true;",
		[
			"data-record-declarations.luau: rule 3 a declaration after the registry seals throws and sealing off allows it again",
		],
	),
	m("3", "guard", RD, "\t\tif (declarations.has(name)) error(`${name} is already declared`, 3);\n", "", [
		"data-record-declarations.luau: rule 3 a duplicate name throws naming it",
	]),
	m(
		"3",
		"guard",
		RD,
		"if (declarations.size() >= MAX_DATA_DECLARATIONS)",
		"if (declarations.size() > MAX_DATA_DECLARATIONS)",
		["data-record-declarations.luau: rule 3 a realm holds at most 256 data declarations"],
	),
	m(
		"3",
		"guard",
		RD,
		"\t\tif (declarations.size() >= MAX_DATA_DECLARATIONS)\n\t\terror(`${name}: more than ${MAX_DATA_DECLARATIONS} data declarations`, 3);\n",
		"",
		["data-record-declarations.luau: rule 3 a realm holds at most 256 data declarations"],
	),
	m("3", "pair", RD, "\tdeclarations.set(name, { name, schema });\n", "", [
		"data-record-declarations.luau: rule 104 a call before binding completes normally when binding happens within its deadline",
	]),
	m(
		"4",
		"swap",
		RD,
		"const schema = sealSchema({ name, version: steps.size(), defaults, node: value, migrations: steps });",
		"const schema = sealSchema({ name, version: steps.size(), defaults, node: value, migrations: [] });",
		["data-record-declarations.luau: rule 4 migrations declared with the store run on an older record, in order"],
	),
	m("4", "guard", RD, "version: steps.size(),", "version: 0,", [
		"data-record-declarations.luau: rule 4 migrations set the schema version a changed record is stored at",
	]),
	m("4", "pair", RD, "\treturn table.freeze({\n\t\tread(key: string) {", "\treturn ({\n\t\tread(key: string) {", [
		"data-record-declarations.luau: rule 3 a declaration after the registry seals throws and sealing off allows it again",
	]),
	m(
		"4",
		"swap",
		RD,
		"return installed(name).read(name, key) as StoreResult<T | undefined>;",
		"return installed(name).read(key, name) as StoreResult<T | undefined>;",
		["data-record-declarations.luau: rule 104 after binding read and update return the public shapes"],
	),
	m(
		"4",
		"swap",
		RD,
		"return installed(name).update(\n\t\t\t\t\tname,\n\t\t\t\t\tkey,",
		"return installed(name).update(\n\t\t\t\t\tkey,\n\t\t\t\t\tname,",
		[
			"data-record-declarations.luau: rule 104 a call before binding completes normally when binding happens within its deadline",
		],
	),
	m(
		"4",
		"pair",
		RD,
		"\t\t\t\t\tchange as unknown as (current: object) => object,\n\t\t\t\t\tid,\n",
		"\t\t\t\t\tchange as unknown as (current: object) => object,\n",
		["data-record-declarations.luau: rule 104 after binding read and update return the public shapes"],
	),
	m(
		"1",
		"guard",
		RD,
		"\tif (driver === undefined) error(`${name}: the data runtime is not loaded; import @trioforge/crucible/server`, 3);\n\treturn driver;",
		"\treturn driver!;",
		[
			"data-record-declarations.luau: rule 1 a declared store used before the server runtime is loaded throws naming the store",
		],
	),
	m(
		"104",
		"pair",
		RB,
		"\tfor (const [name, declaration] of declaredRecords()) bound.set(name, new RecordStore(engine, declaration.schema));\n",
		"",
		[
			"data-record-declarations.luau: rule 104 a call before binding completes normally when binding happens within its deadline",
		],
	),
	m("104", "pair", RB, "\tactivation?.wake();\n", "", [
		"data-record-declarations.luau: rule 104 a call before binding completes normally when binding happens within its deadline",
	]),
	m("104", "pair", RB, "\tbound.clear();\n", "", [
		"data-record-declarations.luau: rule 104 after unbinding a call waits for the next activation",
	]),
	m(
		"104",
		"guard",
		RB,
		"while (bound.get(name) === undefined && bootClock() < deadline) {",
		"while (bound.get(name) === undefined && false as boolean) {",
		[
			"data-record-declarations.luau: rule 104 a call before binding completes normally when binding happens within its deadline",
		],
	),
	m(
		"104",
		"swap",
		RB,
		"const deadline = bootClock() + CALL_DEADLINE_SECONDS;\n\twhile",
		"const deadline = bootClock() + CALL_DEADLINE_SECONDS * 2;\n\twhile",
		[
			"data-record-declarations.luau: rule 104 a call before binding waits for activation and returns timeout at the 30 second deadline",
		],
	),
	m(
		"104",
		"swap",
		RB,
		"const deadline = bootClock() + CALL_DEADLINE_SECONDS;\n\twhile",
		"const deadline = bootClock() - CALL_DEADLINE_SECONDS;\n\twhile",
		[
			"data-record-declarations.luau: rule 104 a call before binding completes normally when binding happens within its deadline",
		],
	),
	m("104", "swap", RB, "activation.park(deadline);", "activation.park();", [
		"data-record-declarations.luau: rule 104 a call before binding waits for activation and returns timeout at the 30 second deadline",
	]),
	m(
		"104",
		"pair",
		RB,
		"\t\tactivation ??= new Waiters(dataPort());\n",
		"\t\tactivation = new Waiters(dataPort());\n",
		["data-record-declarations.luau: rule 104 two calls waiting for activation are both woken by the binding"],
	),
	m(
		"90",
		"swap",
		RB,
		'return result.kind === "ok" ? { ok: true, value: result.value } : { ok: false, reason: result.kind };',
		'return result.kind === "ok" ? { ok: true, value: result.value } : { ok: false, reason: "unavailable" };',
		["data-record-declarations.luau: rule 90 every failure kind maps to its reason string in the public shape"],
	),
	m(
		"90",
		"guard",
		RB,
		"? { ok: true, value: result.value, replayed: result.replayed }",
		"? { ok: true, value: result.value, replayed: !result.replayed }",
		[
			"data-record-declarations.luau: rule 104 a call before binding completes normally when binding happens within its deadline",
		],
	),
	m(
		"90",
		"pair",
		RB,
		"? { ok: true, value: result.value, replayed: result.replayed }",
		"? { ok: true, value: result.value, replayed: false }",
		["data-record-declarations.luau: rule 104 after binding read and update return the public shapes"],
	),
	m(
		"90",
		"swap",
		RB,
		": { ok: false, reason: result.kind };\n}\n\ninstallRecordDriver",
		': { ok: false, reason: "unavailable" };\n}\n\ninstallRecordDriver',
		["data-record-declarations.luau: rule 90 every failure kind maps to its reason string in the public shape"],
	),
	m(
		"104",
		"swap",
		RB,
		'if (store === undefined) return { ok: false, reason: "timeout" };\n\t\treturn publicResult(store.read(key));',
		'if (store === undefined) return { ok: false, reason: "unavailable" };\n\t\treturn publicResult(store.read(key));',
		[
			"data-record-declarations.luau: rule 104 a call before binding waits for activation and returns timeout at the 30 second deadline",
		],
	),
	m(
		"104",
		"swap",
		RB,
		'if (store === undefined) return { ok: false, reason: "timeout" };\n\t\treturn publicUpdate(store.update(key, change, id));',
		'if (store === undefined) return { ok: false, reason: "unavailable" };\n\t\treturn publicUpdate(store.update(key, change, id));',
		[
			"data-record-declarations.luau: rule 104 a call before binding waits for activation and returns timeout at the 30 second deadline",
		],
	),
	m(
		"104",
		"pair",
		RB,
		"return publicUpdate(store.update(key, change, id));",
		"return publicUpdate(store.update(key, change));",
		["data-record-declarations.luau: rule 104 after binding read and update return the public shapes"],
	),
	m("104", "swap", RB, "return publicResult(store.read(key));", "return publicResult(store.read(name));", [
		"data-record-declarations.luau: rule 104 after binding read and update return the public shapes",
	]),
	// The load path in profiles.ts and session.ts.
	m("74", "swap", P, "session.revision = loaded.migrated ? 1 : 0;", "session.revision = loaded.migrated ? 0 : 1;", [
		"data-migrations.luau: rule 18 and 74 a key with no envelope, and one with only inbox entries, take the defaults at version N and run no step",
	]),
	m("74", "guard", P, "session.revision = loaded.migrated ? 1 : 0;", "session.revision = 0;", [
		"data-migrations.luau: rule 27 and 74 an ambiguous commit never stores the migrated payload under version 0 or the old one under 2",
	]),
	m("74", "pair", P, "session.revision = loaded.migrated ? 1 : 0;", "session.revision = 1;", [
		"data-migrations.luau: rule 18 and 74 a key with no envelope, and one with only inbox entries, take the defaults at version N and run no step",
	]),
	m(
		"77",
		"swap",
		P,
		'loaded.kind === "newer" ? SERVER_UPDATING_MESSAGE : LOAD_FAILED_MESSAGE',
		'loaded.kind === "newer" ? LOAD_FAILED_MESSAGE : SERVER_UPDATING_MESSAGE',
		[
			"data-admission.luau: rule 110 unreadable-record diagnostics carry store, user and path, never a value, and a user who rejoins is logged once",
		],
	),
	m(
		"110",
		"guard",
		P,
		'`load:${this.schema.name}:${loaded.kind === "newer" ? "newer" : loaded.code}`',
		"`load:${this.schema.name}:${userId}`",
		["data-migrations.luau: rule 110 a failed migration is diagnosed once per store and code, not once per user"],
	),
	m(
		"110",
		"swap",
		P,
		"`${this.schema.name}: ${userId}'s record did not load: ${loaded.reason}`",
		"`${userId}: ${this.schema.name}'s record did not load: ${loaded.reason}`",
		[
			"data-migrations.luau: rule 75 a step that throws, yields or returns a non-record fails the load at once and leaves the data untouched",
		],
	),
	m(
		"77",
		"pair",
		P,
		'return this.end(session, true, loaded.kind === "newer"',
		'return this.end(session, false, loaded.kind === "newer"',
		[
			"data-admission.luau: rule 110 unreadable-record diagnostics carry store, user and path, never a value, and a user who rejoins is logged once",
		],
	),
	m(
		"76",
		"pair",
		P,
		'return this.end(session, true, loaded.kind === "newer"',
		'return this.end(session, false, loaded.kind === "newer"',
		[
			"data-admission.luau: rule 110 unreadable-record diagnostics carry store, user and path, never a value, and a user who rejoins is logged once",
		],
	),
	m(
		"75",
		"pair",
		P,
		'const loaded = loadRecord(this.schema, envelope.payload, envelope.schemaVersion);\n\t\tif (loaded.kind !== "ok") {',
		'const loaded = loadRecord(this.schema, envelope.payload, this.schema.version);\n\t\tif (loaded.kind !== "ok") {',
		[
			"data-migrations.luau: rule 110 a failed migration is diagnosed once for a user who rejoins, with the store, user and step",
		],
	),
	m(
		"77",
		"swap",
		P,
		'else if (answer.kind === "newer") return this.end(session, false, SERVER_UPDATING_MESSAGE);',
		'else if (answer.kind === "newer") return this.end(session, false, LOAD_FAILED_MESSAGE);',
		["data-migrations.luau: rule 77 and durability sequence an older build meets a newer version"],
	),
	m(
		"77",
		"pair",
		P,
		'else if (answer.kind === "newer") return this.end(session, false, SERVER_UPDATING_MESSAGE);',
		'else if (answer.kind === "newer") return this.end(session, true, SERVER_UPDATING_MESSAGE);',
		["data-migrations.luau: rule 77 and durability sequence an older build meets a newer version"],
	),
	m(
		"110",
		"swap",
		P,
		"`${this.schema.name}: ${userId}'s record is unreadable: ${answer.reason}`",
		"`${userId}: ${this.schema.name}'s record is unreadable: ${answer.reason}`",
		[
			"data-migrations.luau: rule 110 an unreadable envelope is diagnosed once per user with the store and the user, and kicks at once",
		],
	),
	m(
		"77",
		"pair",
		P,
		"this.engine.diagnose(\n\t\t\t\t\t`unreadable:${this.schema.name}:${userId}`,",
		"this.engine.diagnose(\n\t\t\t\t\t`unreadable:${this.schema.name}`,",
		[
			"data-migrations.luau: rule 110 an unreadable envelope is diagnosed once per user with the store and the user, and kicks at once",
		],
	),
	m(
		"77",
		"swap",
		P,
		"return this.end(session, false, LOAD_FAILED_MESSAGE);\n\t\t\t} else {",
		"return this.end(session, false, SERVER_UPDATING_MESSAGE);\n\t\t\t} else {",
		[
			"data-migrations.luau: rule 110 an unreadable envelope is diagnosed once per user with the store and the user, and kicks at once",
		],
	),
	m(
		"49",
		"pair",
		P,
		"const loaded = loadRecord(this.schema, parsed.envelope.payload, parsed.envelope.schemaVersion);\n\t\treturn loaded.kind",
		"const loaded = loadRecord(this.schema, parsed.envelope.payload, this.schema.version);\n\t\treturn loaded.kind",
		[
			"data-migrations.luau: rule 49 inspect of an old-version record returns the migrated value and writes nothing",
		],
	),
	m("74", "pair", S, "\t\t\tschemaVersion: this.schema.version,\n", "\t\t\tschemaVersion: 0,\n", [
		"data-migrations.luau: rule 18 and 74 a key with no envelope, and one with only inbox entries, take the defaults at version N and run no step",
	]),
	m("13", "pair", S, "\t\t\t\tcallbackThreads.add(worker);\n", "", [
		"data-records.luau: rule 13 a generic update inside a profile data callback throws and the write is cancelled",
	]),
	m("13", "pair", S, "\t\tif (worker !== undefined) callbackThreads.delete(worker);\n", "", [
		"data-records.luau: rule 13 a worker that ran a data callback is clean when the pool hands it to the next call",
	]),
	m("13", "guard", S, "\t\tif (status !== CallResult.Ok) error(value, 0);\n", "", [
		"data-records.luau: rule 13 a generic update inside a profile data callback throws and the write is cancelled",
	]),
	m("13", "guard", S, "\tif (callbackThreads.has(coroutine.running())) error(", "\tif (false as boolean) error(", [
		"data-records.luau: rule 13 a generic update inside a profile data callback throws and the write is cancelled",
	]),
	// Operation ids as the generic store uses them.
	m(
		"88",
		"guard",
		O,
		'if (created > now + ID_FUTURE_SKEW_SECONDS) return "future";',
		'if (created > now + ID_FUTURE_SKEW_SECONDS * 2) return "future";',
		[
			"data-records.luau: rule 88 an id created five seconds ahead is accepted and six seconds ahead is expired before any call",
		],
	),
	m("88", "swap", O, "if (created > now + ID_FUTURE_SKEW_SECONDS)", "if (created > now - ID_FUTURE_SKEW_SECONDS)", [
		"data-records.luau: rule 13 a generic update or a profile write inside change throws and does not run",
	]),
	m("88", "swap", O, "if (now - created > maxAgeSeconds)", "if (created - now > maxAgeSeconds)", [
		"data-records.luau: rule 88 an id older than 3,600 seconds is expired before any backend call and the boundary is exact",
	]),
	m(
		"88",
		"swap",
		O,
		"return seconds === undefined ? undefined : tonumber(seconds, 36);",
		"return seconds === undefined ? undefined : tonumber(seconds, 10);",
		[
			"data-record-declarations.luau: rule 104 a call before binding completes normally when binding happens within its deadline",
		],
	),
	m("88", "guard", O, '"^o1%-([0-9a-z]+)%-[0-9a-z]+$"', '"o1%-([0-9a-z]+)%-[0-9a-z]+$"', [
		"data-records.luau: rule 88 an id over 64 bytes or not made by operationId throws at the call naming the store and sends nothing",
	]),
	m("88", "guard", O, '"^o1%-([0-9a-z]+)%-[0-9a-z]+$"', '"^o1%-([0-9a-z]+)%-[0-9a-z]+"', [
		"data-records.luau: rule 88 an id over 64 bytes or not made by operationId throws at the call naming the store and sends nothing",
	]),
	m("88", "guard", O, '"^o1%-([0-9a-z]+)%-[0-9a-z]+$"', '"^o1%-([0-9a-z]+)%-[0-9a-z]*$"', [
		"data-records.luau: rule 88 an id over 64 bytes or not made by operationId throws at the call naming the store and sends nothing",
	]),
	m("88", "guard", O, '"^o1%-([0-9a-z]+)%-[0-9a-z]+$"', '"^o1%-([0-9a-z]+)%-[0-9a-z]+.*$"', [
		"data-records.luau: rule 88 an id over 64 bytes or not made by operationId throws at the call naming the store and sends nothing",
	]),
	m(
		"88",
		"swap",
		O,
		'const [guid] = port.guid().lower().gsub("[^%w]", "");',
		'const [guid] = port.guid().gsub("[^%w]", "");',
		[
			"data-record-declarations.luau: rule 66 operationId writes the wall second in base 36 and the GUID in lower case",
		],
	),
	m(
		"88",
		"swap",
		O,
		"return created === undefined || now - created > ID_EVICT_MIN_AGE_SECONDS;",
		"return created === undefined || now - created > ID_EVICT_MIN_AGE_SECONDS - 1;",
		["data-inbox-pins.luau: A16 rule 69 a seen ring's head is refused at 59.9 seconds old and accepted at 60.1"],
	),
	m(
		"88",
		"guard",
		O,
		"return created === undefined || now - created > ID_EVICT_MIN_AGE_SECONDS;",
		"return now - created! > ID_EVICT_MIN_AGE_SECONDS;",
		[
			"data-records.luau: rule 88 a full ring whose oldest entry is not an operation id evicts it and leaves the floor",
		],
	),
	m(
		"88",
		"swap",
		O,
		"return created === undefined || now - created > ID_EVICT_MIN_AGE_SECONDS;",
		"return created === undefined || created - now > ID_EVICT_MIN_AGE_SECONDS;",
		["data-inbox-pins.luau: A16 rule 69 a seen ring's head is refused at 59.9 seconds old and accepted at 60.1"],
	),
	m("88", "swap", O, "return `o1-${base36(port.wall())}-${guid}`;", "return `o1-${guid}-${base36(port.wall())}`;", [
		"data-record-declarations.luau: rule 66 operationId makes o1, the base 36 wall second and an alphanumeric token within 64 bytes",
	]),
	m("88", "guard", O, "const digit = rest % 36;", "const digit = rest % 35;", [
		"data-record-declarations.luau: rule 66 operationId makes o1, the base 36 wall second and an alphanumeric token within 64 bytes",
	]),
	m(
		"88",
		"swap",
		O,
		"text = DIGITS.sub(digit + 1, digit + 1) + text;",
		"text = text + DIGITS.sub(digit + 1, digit + 1);",
		[
			"data-record-declarations.luau: rule 66 operationId makes o1, the base 36 wall second and an alphanumeric token within 64 bytes",
		],
	),
	m("88", "guard", O, "rest = math.floor(rest / 36);", "rest = math.floor(rest / 35);", [
		"data-record-declarations.luau: rule 66 operationId makes o1, the base 36 wall second and an alphanumeric token within 64 bytes",
	]),
	m("88", "guard", O, "} while (rest > 0);", "} while (rest > 1);", [
		"data-record-declarations.luau: rule 66 operationId writes the wall second in base 36 and the GUID in lower case",
	]),
	// Amendment A16: the four D2 sites, each pointed at its pin.
	m(
		"72",
		"guard",
		I,
		"if (revision <= session.confirmed) session.acks.delete(id);",
		"if (revision < session.confirmed) session.acks.delete(id);",
		[
			"data-inbox-pins.luau: A16 rule 72 an ack at exactly the confirmed revision is retired by the envelope that confirms it",
		],
	),
	m(
		"70",
		"guard",
		I,
		"\t\t\t\t\titem?.generation === mark.generation + 1 &&\n\t\t\t\t\titem.attempts === mark.attempts\n",
		"\t\t\t\t\titem?.generation === mark.generation + 1\n",
		[
			"data-inbox-pins.luau: A16 rule 70 a quarantine mark whose item moved by one generation and a different attempt count is not a full quarantine",
		],
	),
	m(
		"70",
		"swap",
		I,
		"item?.generation === mark.generation + 1 &&\n\t\t\t\t\titem.attempts === mark.attempts",
		"item?.generation === mark.generation + 1 &&\n\t\t\t\t\titem.attempts === mark.generation",
		[
			"data-inbox-pins.luau: A16 rule 70 a quarantine mark whose item moved by one generation and a different attempt count is not a full quarantine",
		],
	),
	m(
		"71",
		"guard",
		I,
		"if (session.userId === userId && session.phase === Phase.Active)",
		"if (session.userId === userId)",
		[
			"data-inbox-pins.luau: A16 rule 71 a send answered while the user's session on this server is sealed requests no write",
		],
	),
	m(
		"69",
		"guard",
		O,
		"return created === undefined || now - created > ID_EVICT_MIN_AGE_SECONDS;",
		"return created === undefined || now - created > ID_EVICT_MIN_AGE_SECONDS + 1;",
		["data-inbox-pins.luau: A16 rule 69 a seen ring's head is refused at 59.9 seconds old and accepted at 60.1"],
	),
];
// Type-checks every mutant in one language service, so a mutant that cannot compile fails before the slow runs.
function typecheck(originals) {
	const ts = require(require.resolve("typescript", { paths: [path.join(scratch, "packages/crucible")] }));
	const configPath = path.join(scratch, "packages/crucible/tsconfig.json");
	const config = ts.parseJsonConfigFileContent(
		ts.readConfigFile(configPath, ts.sys.readFile).config,
		ts.sys,
		path.dirname(configPath),
	);
	const texts = new Map();
	const versions = new Map();
	const service = ts.createLanguageService({
		getScriptFileNames: () => config.fileNames,
		getCompilationSettings: () => config.options,
		getScriptVersion: (file) => String(versions.get(path.resolve(file)) ?? 0),
		getScriptSnapshot: (file) => {
			const text =
				texts.get(path.resolve(file)) ?? (fs.existsSync(file) ? fs.readFileSync(file, "utf8") : undefined);
			return text === undefined ? undefined : ts.ScriptSnapshot.fromString(text);
		},
		getCurrentDirectory: () => path.dirname(configPath),
		getDefaultLibFileName: (options) => ts.getDefaultLibFilePath(options),
		fileExists: ts.sys.fileExists,
		readFile: ts.sys.readFile,
		readDirectory: ts.sys.readDirectory,
		directoryExists: ts.sys.directoryExists,
		getDirectories: ts.sys.getDirectories,
	});
	const errors = (file) =>
		[...service.getSyntacticDiagnostics(file), ...service.getSemanticDiagnostics(file)].map((d) =>
			ts.flattenDiagnosticMessageText(d.messageText, " "),
		);
	const baseline = new Map();
	const broken = [];
	for (const [index, [rule, kind, file, before, after]] of mutants.entries()) {
		if (!file.endsWith(".ts")) continue;
		const absolute = path.resolve(scratch, file);
		if (!baseline.has(absolute)) baseline.set(absolute, errors(absolute).length);
		texts.set(absolute, apply(originals.get(file), before, after));
		versions.set(absolute, (versions.get(absolute) ?? 0) + 1);
		const found = errors(absolute);
		texts.delete(absolute);
		versions.set(absolute, versions.get(absolute) + 1);
		if (found.length > baseline.get(absolute))
			broken.push(`${index} rule ${rule} ${kind} does not compile: ${found[0]}`);
	}
	assert.deepEqual(broken, [], "every mutant compiles");
}
function run(command, args, timeout = 600000) {
	const result = spawnSync(command, args, { cwd: scratch, encoding: "utf8", timeout });
	assert.equal(result.error, undefined, String(result.error));
	return result;
}
const targetsOf = (target) => (Array.isArray(target) ? target : [target]);
// A mutant that hangs its spec is detected, so a timed-out spec counts as killed.
function spec(target) {
	const result = spawnSync("lune", ["run", "test", ...targetsOf(target)], {
		cwd: scratch,
		encoding: "utf8",
		timeout: 180000,
	});
	if (result.error?.code === "ETIMEDOUT") return { hung: true, output: "" };
	assert.equal(result.error, undefined, String(result.error));
	return { hung: false, status: result.status, output: result.stdout + result.stderr };
}
if (process.env.SHOW !== undefined) {
	for (const index of process.env.SHOW.split(",").map(Number))
		console.log(index, JSON.stringify(mutants[index].slice(0, 5), null, 1));
	process.exit(0);
}
try {
	for (const line of K3) console.log(`K3 ${line}`);
	for (const name of ["tsconfig.rbxts.json", ".luaurc", "rokit.toml"])
		fs.copyFileSync(path.join(repo, name), path.join(scratch, name));
	fs.cpSync(path.join(repo, ".lune"), path.join(scratch, ".lune"), { recursive: true });
	fs.cpSync(path.join(repo, "test"), path.join(scratch, "test"), { recursive: true });
	fs.cpSync(path.join(repo, "packages/crucible"), path.join(scratch, "packages/crucible"), {
		recursive: true,
		filter: (p) => !p.split(path.sep).includes("node_modules"),
	});
	fs.symlinkSync(path.join(repo, "node_modules"), path.join(scratch, "node_modules"), "junction");
	fs.symlinkSync(
		path.join(repo, "packages/crucible/node_modules"),
		path.join(scratch, "packages/crucible/node_modules"),
		"junction",
	);
	const files = [...new Set(mutants.map(([, , file]) => file))];
	const originals = new Map(files.map((file) => [file, fs.readFileSync(path.join(scratch, file), "utf8")]));
	const unmatched = [];
	for (const [index, [rule, kind, file, before]] of mutants.entries()) {
		const count = occurrences(originals.get(file), before);
		if (count !== 1) unmatched.push(`${index} rule ${rule} ${kind}: ${count} matches in ${file}: ${before}`);
	}
	assert.deepEqual(unmatched, [], "every pattern occurs once");
	typecheck(originals);
	if (process.env.DRY !== undefined) {
		console.log(`${mutants.length} mutants match once and compile`);
		process.exit(0);
	}
	const build = () =>
		run(process.execPath, [
			require.resolve("roblox-ts/out/CLI/cli.js"),
			"--type",
			"package",
			"-p",
			"packages/crucible",
		]);
	assert.equal(build().status, 0, "baseline build");
	const targets = [...new Set(mutants.flatMap((row) => targetsOf(row[5])))];
	const baseline = run("lune", ["run", "test", ...targets]);
	assert.equal(baseline.status, 0, baseline.stdout + baseline.stderr);
	const start = Number(process.argv[2] ?? 0);
	const only = process.env.ONLY?.split(",").map(Number);
	const end = Number(process.env.END ?? mutants.length);
	let killed = 0;
	const survivors = [];
	for (const [index, [rule, kind, file, before, after, target]] of mutants.entries()) {
		if (index < start || index >= end || (only !== undefined && !only.includes(index))) continue;
		const original = originals.get(file);
		fs.writeFileSync(path.join(scratch, file), apply(original, before, after));
		const compiled = build();
		assert.equal(
			compiled.status,
			0,
			`${index} rule ${rule} ${kind} build failed\n${compiled.stdout}${compiled.stderr}`,
		);
		const tested = spec(target);
		fs.writeFileSync(path.join(scratch, file), original);
		const output = tested.output;
		const failed = output.split("\n").find((line) => line.startsWith("FAIL "));
		const label = targetsOf(target).join(" + ");
		const cutShort = !tested.hung && !/[0-9]+ passed, [0-9]+ failed/.test(output);
		if (
			!tested.hung &&
			!cutShort &&
			(tested.status === 0 || failed === undefined || !targetsOf(target).some((t) => output.includes(t)))
		) {
			survivors.push(`${index} rule ${rule} ${kind}`);
			console.log(`SURVIVED ${index} | rule ${rule} | ${kind} | ${label}`);
			if (process.env.VERBOSE !== undefined) console.log(`status ${tested.status}`, output.slice(-600));
			continue;
		}
		console.log(
			`KILLED ${index} | rule ${rule} | ${kind} | ${label} | ${tested.hung ? "hung" : (failed ?? "no summary")}`,
		);
		killed++;
	}
	assert.equal(build().status, 0, "restored build");
	console.log(
		`${killed}/${only?.length ?? Math.min(end, mutants.length) - start} migration, admission and record mutants killed; restored build 0`,
	);
	assert.deepEqual(survivors, [], "every mutant is killed");
} finally {
	fs.rmSync(scratch, { recursive: true, force: true });
}
