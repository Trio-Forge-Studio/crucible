const fs = require("node:fs");
const path = require("node:path");

function packageOf(file) {
	let dir = path.dirname(path.resolve(file));
	for (;;) {
		const manifest = path.join(dir, "package.json");
		if (fs.existsSync(manifest)) return { dir, json: JSON.parse(fs.readFileSync(manifest, "utf8")) };
		const parent = path.dirname(dir);
		if (parent === dir) return undefined;
		dir = parent;
	}
}

function typesCondition(value) {
	if (!value || typeof value !== "object") return [];
	if (typeof value.types === "string") return [value.types];
	return Object.values(value).flatMap(typesCondition);
}

function typeId(file, name, program, ts) {
	const owner = packageOf(file);
	if (!owner?.json.name) return undefined;
	let root;
	if (/\.d\.tsx?$/.test(file)) {
		const entries = typesCondition(owner.json.exports?.["."] ?? owner.json.exports);
		const entry =
			entries.find((entry) => {
				const relative = path.relative(path.dirname(path.resolve(owner.dir, entry)), file);
				return !relative.startsWith("..") && !path.isAbsolute(relative);
			}) ??
			owner.json.types ??
			owner.json.typings ??
			"index.d.ts";
		root = path.dirname(path.resolve(owner.dir, entry));
	} else {
		const config = ts.findConfigFile(path.dirname(file), ts.sys.fileExists);
		if (config && packageOf(config)?.dir === owner.dir) {
			const parsed = ts.getParsedCommandLineOfConfigFile(
				config,
				{},
				{ ...ts.sys, onUnRecoverableConfigFileDiagnostic() {} },
			);
			root = parsed?.options.rootDir;
		}
		root ??= program.getCompilerOptions().rootDir;
		root ??= owner.dir;
	}
	const modulePath = path
		.relative(root, file)
		.split(path.sep)
		.join("/")
		.replace(/(\.d)?\.tsx?$/, "");
	if (modulePath.startsWith("../") || path.isAbsolute(modulePath)) return undefined;
	return `${owner.json.name}/${modulePath}:${name}`;
}

module.exports = { packageOf, typeId };
