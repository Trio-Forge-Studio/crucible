import { spawn } from "node:child_process";
import readline from "node:readline";

export async function connect() {
	const child = spawn("cmd.exe", ["/c", `${process.env.LOCALAPPDATA}/Roblox/mcp.bat`], { windowsHide: true });
	child.stderr.on("data", () => {});
	let nextId = 0;
	const pending = new Map();
	readline.createInterface({ input: child.stdout }).on("line", (line) => {
		let response;
		try {
			response = JSON.parse(line);
		} catch {
			return;
		}
		const resolve = pending.get(response.id);
		if (resolve) {
			pending.delete(response.id);
			resolve(response);
		}
	});
	const request = async (method, params, timeout = 45000) => {
		const id = ++nextId;
		const response = await new Promise((resolve, reject) => {
			const timer = setTimeout(() => {
				pending.delete(id);
				reject(Error(`${method} timeout`));
			}, timeout);
			pending.set(id, (value) => {
				clearTimeout(timer);
				resolve(value);
			});
			child.stdin.write(JSON.stringify({ jsonrpc: "2.0", id, method, params }) + "\n");
		});
		if (response.error) throw Error(JSON.stringify(response.error));
		return response.result;
	};
	await request("initialize", {
		protocolVersion: "2025-06-18",
		capabilities: {},
		clientInfo: { name: "crucible-g1-probes", version: "1" },
	});
	child.stdin.write(JSON.stringify({ jsonrpc: "2.0", method: "notifications/initialized" }) + "\n");
	return {
		request,
		close() {
			child.stdin.end();
			child.kill();
		},
	};
}

export function text(result) {
	return result.content
		.filter((item) => item.type === "text")
		.map((item) => item.text)
		.join("\n");
}
