import fs from "node:fs";
import path from "node:path";
import { spawn } from "node:child_process";
import { setTimeout as delay } from "node:timers/promises";
import { retryStartup } from "./startup.mjs";

const [behavior = "Deferred", shutdown = "short"] = process.argv.slice(2);
const label = `${behavior.toLowerCase()}-${shutdown}`;
const results = path.join(import.meta.dirname, "results");
process.exitCode = await retryStartup(async (attempt) => {
	const child = spawn(process.execPath, [path.join(import.meta.dirname, "driver.mjs"), behavior, shutdown], {
		stdio: "inherit",
	});
	const code = await new Promise((resolve, reject) => {
		child.once("error", reject);
		child.once("exit", (code) => resolve(code ?? 1));
	});
	const sessionFile = path.join(results, `${label}-session.json`);
	const session = fs.existsSync(sessionFile) ? JSON.parse(fs.readFileSync(sessionFile, "utf8")) : undefined;
	const startupCrash =
		code !== 0 &&
		session?.driverPid === child.pid &&
		session?.error === "Error: Owned Studio exited during startup";
	if (startupCrash) {
		for (const suffix of ["session.json", "native.log"]) {
			const file = path.join(results, `${label}-${suffix}`);
			if (fs.existsSync(file)) fs.copyFileSync(file, path.join(results, `${label}-crash-${attempt}-${suffix}`));
		}
	}
	return { code, startupCrash };
}, delay);
