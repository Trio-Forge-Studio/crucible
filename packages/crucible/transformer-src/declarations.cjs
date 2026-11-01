const limits = Object.freeze({
	MESSAGE_NAME_MAX_BYTES: 64,
	MAX_ASK_TIMEOUT_SECONDS: 120,
	MAX_UNION_BRANCHES: 256,
	MAX_SCHEMA_NODES: 4096,
	MAX_DEPTH: 32,
});
const reactiveLimits = Object.freeze({
	MESSAGE_NAME_MAX_BYTES: 64,
	SYNC_VALUE_DEPTH: 28,
});
const dataLimits = Object.freeze({
	STORE_NAME_MAX_BYTES: 50,
	MESSAGE_NAME_MAX_BYTES: 64,
	CONFIG_NAME_MAX_BYTES: 64,
	PERSIST_DEPTH: 28,
	VIEW_NAME_PREFIX: "Crucible.Data.",
	LIVE_NAME_PREFIX: "Crucible.Live.",
	VIEW_NAME_MAX_BYTES: 64,
	RESERVED_HANDLE_NAMES: Object.freeze(["set", "update", "insert", "remove"]),
});

const message = {
	label: "messages",
	side: "shared",
	names: "Message",
	maxBytes: limits.MESSAGE_NAME_MAX_BYTES,
	typeArguments: 1,
	arguments: 1,
	passed: [],
	position: "payload",
};
const replicated = {
	label: "replicated state",
	side: "shared",
	names: "Replicated",
	maxBytes: reactiveLimits.MESSAGE_NAME_MAX_BYTES,
	typeArguments: 1,
	arguments: 2,
	passed: [1],
	position: "state",
};
const config = {
	label: "configs",
	side: "server",
	names: "Config",
	maxBytes: dataLimits.CONFIG_NAME_MAX_BYTES,
	typeArguments: 1,
	arguments: 2,
	passed: [1],
	position: "persist",
};
const kinds = Object.freeze({
	toServer: message,
	toClient: message,
	askServer: { ...message, typeArguments: 2, arguments: 2 },
	replicated,
	replicatedPerPlayer: replicated,
	profileStore: {
		label: "profile stores",
		side: "shared",
		names: "Store",
		maxBytes: dataLimits.STORE_NAME_MAX_BYTES,
		typeArguments: "optional",
		arguments: 2,
		passed: [1],
		position: "persist",
		store: true,
		handles: true,
	},
	dataStore: {
		label: "data stores",
		side: "server",
		names: "Store",
		maxBytes: dataLimits.STORE_NAME_MAX_BYTES,
		typeArguments: 1,
		arguments: 2,
		passed: [1],
		position: "persist",
		store: true,
	},
	profileMessage: {
		label: "profile messages",
		side: "server",
		names: "Profile message",
		maxBytes: dataLimits.MESSAGE_NAME_MAX_BYTES,
		typeArguments: 1,
		arguments: 1,
		passed: [],
		position: "persist",
	},
	config,
	playerConfig: config,
});
const depthLimits = Object.freeze({
	payload: limits.MAX_DEPTH,
	state: reactiveLimits.SYNC_VALUE_DEPTH,
	persist: dataLimits.PERSIST_DEPTH,
});

const byBytes = (a, b) => Buffer.compare(Buffer.from(a), Buffer.from(b));
const byPath = (a, b) => {
	for (let i = 0; i < Math.min(a.length, b.length); i++) {
		const order = byBytes(a[i], b[i]);
		if (order) return order;
	}
	return a.length - b.length;
};
// Marks sit only on object fields from the root, so only object fields can lose every field.
function replica(node) {
	if (node.kind !== "object") return node;
	const fields = [];
	for (const [key, child] of node.fields) {
		if (child.serverOnly) continue;
		const kept = replica(child);
		if (kept.kind === "object" && !kept.fields.length) continue;
		fields.push([key, kept]);
	}
	return { kind: "object", fields };
}
function serverOnlyPaths(node, prefix = []) {
	if (node.kind !== "object") return [];
	return node.fields
		.flatMap(([key, child]) => (child.serverOnly ? [[...prefix, key]] : serverOnlyPaths(child, [...prefix, key])))
		.sort(byPath);
}
const SEGMENT = /^[A-Za-z][A-Za-z0-9_]*$/;
function viewNameProblems(store, node) {
	if (node.kind !== "object") return [];
	const prefix = `${dataLimits.VIEW_NAME_PREFIX}${store}.`;
	return node.fields.flatMap(([field]) => {
		const name = prefix + field;
		if (!SEGMENT.test(field))
			return [
				[
					`value[${JSON.stringify(field)}]`,
					"a field the owner's client receives is named by one declaration segment, [A-Za-z][A-Za-z0-9_]*; rename it or mark it serverOnly",
				],
			];
		if (Buffer.byteLength(name) > dataLimits.VIEW_NAME_MAX_BYTES)
			return [
				[
					`value.${field}`,
					`its view name ${name} is ${Buffer.byteLength(name)} bytes, over ${dataLimits.VIEW_NAME_MAX_BYTES}; this store allows field names of at most ${dataLimits.VIEW_NAME_MAX_BYTES - Buffer.byteLength(prefix)} bytes; rename the field, shorten the store name, or mark it serverOnly`,
				],
			];
		return [];
	});
}

module.exports = {
	limits,
	reactiveLimits,
	dataLimits,
	kinds,
	depthLimits,
	byBytes,
	replica,
	serverOnlyPaths,
	viewNameProblems,
};
