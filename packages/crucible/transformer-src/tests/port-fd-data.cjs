// Run: node port-fd-data.cjs [extracted FD src parent]. Without an argument it reads FD with git show origin/master.
const fs = require("node:fs");
const path = require("node:path");
const { spawnSync } = require("node:child_process");
const ts = require("typescript");
const crypto = require("node:crypto");
const sources = {
	"src/core/Input/shared/keybinds.ts": ["rebindableActions", "RebindableAction", "Keybinds"],
	"src/core/Career/shared/career.ts": ["ItemTallies", "JobRecord", "Career", "PlayedAnchor"],
	"src/core/Data/shared/template.ts": [
		"VolumeChannel",
		"ButtonPrompts",
		"FreeCursorMode",
		"GraphicsQuality",
		"PlayerSettings",
		"DailyRewardHistory",
		"EntranceSettings",
		"CoreData",
		"PlayerData",
		"OwnerData",
	],
	"src/core/Admin/server/data-edits.ts": ["AdminEdit"],
	"src/core/Commands/shared/command-types.ts": ["LogEntry"],
	"src/core/Commands/server/audit-buffer.ts": ["SavedEntry"],
};
const extracted = process.argv[2];
const read = (file) => {
	if (extracted) return fs.readFileSync(path.join(extracted, file), "utf8");
	const result = spawnSync(
		"git",
		["-C", "C:/Users/Admin/dev/trio-forge/fragile-delivery", "show", `origin/master:${file}`],
		{ encoding: "utf8" },
	);
	if (result.status !== 0) throw new Error(result.stderr);
	return result.stdout;
};
// serio's f64 is not installed here; any number brand that is not Crucible's width brand is foreign (rule 7).
let fixture = 'declare const serio: unique symbol;\nexport type f64 = number & { readonly [serio]: "f64" };\n';
const provenance = [];
for (const [file, names] of Object.entries(sources)) {
	const text = read(file);
	const sf = ts.createSourceFile(file, text, ts.ScriptTarget.Latest, true);
	for (const name of names) {
		const decl = sf.statements.find(
			(s) =>
				((ts.isInterfaceDeclaration(s) || ts.isTypeAliasDeclaration(s)) && s.name.text === name) ||
				(ts.isVariableStatement(s) && s.declarationList.declarations.some((d) => d.name.text === name)),
		);
		if (!decl) throw new Error(`${file} lacks ${name}`);
		fixture += ts.createPrinter({ removeComments: true }).printNode(ts.EmitHint.Unspecified, decl, sf) + "\n";
	}
	provenance.push({
		file,
		declarations: names,
		sha256: crypto.createHash("sha256").update(text).digest("hex"),
	});
}
fs.writeFileSync(path.join(__dirname, "fixtures/state-data/fd.ts"), fixture);
fs.writeFileSync(
	path.join(__dirname, "fixtures/state-data/fd-sources.json"),
	JSON.stringify(
		{ source: extracted ? "data round extraction" : "git show origin/master", files: provenance },
		null,
		"\t",
	) + "\n",
);
console.log(`Ported ${provenance.reduce((n, s) => n + s.declarations.length, 0)} FD declarations.`);
