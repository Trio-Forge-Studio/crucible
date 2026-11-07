import { defineConfig } from "vite-plus";

export default defineConfig({
	staged: {
		"*": "vp check --fix",
		"{.lune,test,crates/compiler/tests}/**/*.luau": ["stylua", "selene"],
		"probes/**/*.luau": ["stylua", "selene --config probes/engine/selene.toml"],
	},
	fmt: {
		printWidth: 120,
		tabWidth: 4,
		useTabs: true,
		trailingComma: "all",
		sortPackageJson: false,
		ignorePatterns: [
			"**/out/**",
			"**/include/**",
			"build/**",
			"pnpm-lock.yaml",
			"**/*.md",
			"crates/oc-registry/spec/**",
			"crates/cli/notices.hbs",
			"target/**",
		],
		overrides: [
			{
				files: ["**/*.yml", "**/*.yaml"],
				options: { tabWidth: 2, useTabs: false },
			},
		],
	},
	lint: {
		categories: { correctness: "error", suspicious: "error" },
		plugins: ["typescript", "node"],
		rules: { "no-shadow": "off" },
		ignorePatterns: ["**/out/**", "**/include/**", "build/**", "target/**", "packages/compiler-types/types/**"],
	},
});
