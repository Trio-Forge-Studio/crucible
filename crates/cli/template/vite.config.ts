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
		ignorePatterns: ["**/out/**", "**/include/**", "build/**"],
	},
});
