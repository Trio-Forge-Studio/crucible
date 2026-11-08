import { defineConfig } from "vite-plus";

export default defineConfig({
	fmt: {
		useTabs: true,
		tabWidth: 4,
		printWidth: 120,
		sortPackageJson: false,
		ignorePatterns: [
			"**/out/**",
			"**/include/**",
			"build/**",
			"**/*.md",
			"package-lock.json",
			"pnpm-lock.yaml",
			"yarn.lock",
			"bun.lock",
		],
	},
	lint: {
		categories: { correctness: "error", suspicious: "error" },
		plugins: ["typescript"],
		// JSX compiles to Vide.jsx calls, which the linter does not see.
		rules: { "no-unused-vars": ["error", { varsIgnorePattern: "^Vide$" }] },
		ignorePatterns: ["**/out/**", "**/include/**", "build/**"],
	},
});
