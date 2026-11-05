const H = 'import { Subsystem } from "@trioforge/crucible";\n';
const both =
	H + "export class U extends Subsystem { protected override preRender() {} protected override onShutdown() {} }";
const shutdownOnly = H + "export class S extends Subsystem { protected override onShutdown() {} }";
const renderOnly = H + "export class C extends Subsystem { protected override preRender() {} }";
const modes = {
	explicit: {},
	inferred: { rootDir: null },
	rootDirs: { rootDir: null, rootDirs: ["src"] },
	rootDirsWide: { rootDir: null, rootDirs: ["src", "generated"] },
};
const cases = [];
for (const ancestor of ["client", "server", "shared"])
	for (const [mode, compilerOptions] of Object.entries(modes)) {
		const include = mode === "rootDirsWide" ? ["src", "generated"] : undefined;
		const extra = mode === "rootDirsWide" ? { "generated/keep.ts": "export const keep = 1;\n" } : {};
		cases.push({
			name: `${ancestor}/${mode} unmarked reports both`,
			ancestor,
			compilerOptions,
			include,
			files: { "src/unmarked.ts": both, ...extra },
			expect: [
				["CRU011", 2, "src/unmarked.ts"],
				["CRU016", 2, "src/unmarked.ts"],
			],
			exact: true,
		});
		cases.push({
			name: `${ancestor}/${mode} server folder keeps server, client keeps client`,
			ancestor,
			compilerOptions,
			include,
			files: {
				"src/server/s.ts": shutdownOnly,
				"src/client/c.ts": renderOnly,
				"src/server/x.ts": renderOnly.replace("class C", "class X"),
				"src/client/y.ts": shutdownOnly.replace("class S", "class Y"),
				...extra,
			},
			expect: [
				["CRU011", 2, "src/server/x.ts"],
				["CRU016", 2, "src/client/y.ts"],
			],
			exact: true,
		});
	}
cases.push({
	name: "inferred root inside src/server keeps server side",
	ancestor: "client",
	compilerOptions: { rootDir: null },
	files: { "src/server/s.ts": shutdownOnly, "src/server/x.ts": renderOnly.replace("class C", "class X") },
	expect: [["CRU011", 2, "src/server/x.ts"]],
	exact: true,
});
cases.push({
	name: "inferred root inside src/client keeps client side",
	ancestor: "server",
	compilerOptions: { rootDir: null },
	files: { "src/client/c.ts": renderOnly, "src/client/y.ts": shutdownOnly.replace("class S", "class Y") },
	expect: [["CRU016", 2, "src/client/y.ts"]],
	exact: true,
});
cases.push({
	name: "rootDirs src/server only keeps server side",
	ancestor: "client",
	compilerOptions: { rootDir: null, rootDirs: ["src/server"] },
	files: { "src/server/s.ts": shutdownOnly, "src/server/x.ts": renderOnly.replace("class C", "class X") },
	expect: [["CRU011", 2, "src/server/x.ts"]],
	exact: true,
});
cases.push({
	name: "nested nearest segment server/client/a.ts is client",
	ancestor: "server",
	compilerOptions: {},
	files: { "src/server/client/a.ts": renderOnly, "src/client/server/b.ts": shutdownOnly },
	exact: true,
	expect: [],
});
module.exports = cases;
