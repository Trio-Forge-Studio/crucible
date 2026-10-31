import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { createServer } from "node:http";
import { parentPort, workerData } from "node:worker_threads";

// Serves workerData.packages and passes every other package through to npmjs, so a
// game installs its toolchain as usual and only the Crucible packages come from here.
const { files, packages } = workerData;

const server = createServer(async (request, response) => {
	const path = decodeURIComponent(new URL(request.url, "http://registry").pathname.slice(1));
	const tarball = /^tarballs\/(\d+)$/.exec(path);
	if (tarball) {
		response.end(readFileSync(files[Number(tarball[1])]));
		return;
	}
	const versions = packages[path];
	if (!versions) {
		const upstream = await fetch(`https://registry.npmjs.org/${path.replace("/", "%2f")}`, {
			headers: { accept: request.headers.accept ?? "application/json" },
		});
		response.writeHead(upstream.status, {
			"content-type": upstream.headers.get("content-type") ?? "application/json",
		});
		response.end(Buffer.from(await upstream.arrayBuffer()));
		return;
	}
	const origin = `http://${request.headers.host}`;
	const packument = { name: path, "dist-tags": { latest: Object.keys(versions).at(-1) }, versions: {} };
	for (const [version, { manifest, file }] of Object.entries(versions)) {
		const bytes = readFileSync(files[file]);
		packument.versions[version] = {
			...manifest,
			dist: {
				tarball: `${origin}/tarballs/${file}`,
				shasum: createHash("sha1").update(bytes).digest("hex"),
				integrity: `sha512-${createHash("sha512").update(bytes).digest("base64")}`,
			},
		};
	}
	response.writeHead(200, { "content-type": "application/json" });
	response.end(JSON.stringify(packument));
});

server.listen(0, "127.0.0.1", () => parentPort.postMessage(server.address().port));
