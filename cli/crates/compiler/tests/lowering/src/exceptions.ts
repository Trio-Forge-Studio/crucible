const marker = {};

export function isMarker(value: unknown) {
	return value === marker;
}

export function caught(kind: number): unknown {
	try {
		if (kind === 0) throw "plain";
		if (kind === 1) throw marker;
		if (kind === 2) throw 42;
		if (kind === 3) error("raised", 0);
		return "none";
	} catch (e) {
		return e;
	}
}

export function swallow(f: () => void) {
	let done = false;
	try {
		f();
		done = true;
	} catch {}
	return done;
}

export function finallyPaths(path: string) {
	const log = new Array<string>();
	const result = run(path, log);
	return `${result}|${log.join(",")}`;
}

function run(path: string, log: string[]) {
	try {
		log.push("try");
		if (path === "return") return "returned";
		if (path === "throw") throw "thrown";
		log.push("end");
	} catch (e) {
		log.push(`catch ${e}`);
		if (path === "throw") return "caught";
	} finally {
		log.push("finally");
	}
	return "fell";
}

export function loopExits() {
	const log = new Array<string>();
	for (let i = 0; i < 5; i++) {
		try {
			if (i === 1) continue;
			if (i === 3) break;
			log.push(`body ${i}`);
		} finally {
			log.push(`finally ${i}`);
		}
	}
	return log.join(",");
}

export function catchExits(values: number[]) {
	const log = new Array<string>();
	for (const v of values) {
		try {
			if (v < 0) throw v;
			log.push(`ok ${v}`);
		} catch (e) {
			if (e === -1) continue;
			if (e === -2) break;
			log.push(`caught ${e}`);
		} finally {
			log.push(`finally ${v}`);
		}
	}
	return log.join(",");
}

export function switchBreak(n: number) {
	const log = new Array<string>();
	switch (n) {
		case 1:
			try {
				log.push("one");
				break;
			} finally {
				log.push("finally");
			}
		default:
			log.push("other");
	}
	return log.join(",");
}

export function throughFinally(value: unknown) {
	const log = new Array<string>();
	try {
		try {
			throw value;
		} finally {
			log.push("finally");
		}
	} catch (e) {
		return log.size() === 1 ? e : undefined;
	}
}

export function rethrown(value: unknown) {
	let seen: unknown;
	try {
		try {
			throw value;
		} catch (e) {
			seen = e;
			throw e;
		}
	} catch (outer) {
		return seen === outer ? outer : undefined;
	}
}

export function catchThrows(log: string[]) {
	try {
		throw "first";
	} catch {
		throw "second";
	} finally {
		log.push("finally");
	}
}

export function finallyReplacesThrow() {
	try {
		throw "lost";
	} finally {
		// oxlint-disable-next-line no-unsafe-finally
		return "finally";
	}
}

export function finallyReplacesReturn() {
	try {
		return "try";
	} finally {
		// oxlint-disable-next-line no-unsafe-finally
		return "finally";
	}
}

export function nestedReturn(log: string[]) {
	try {
		try {
			return 7;
		} finally {
			log.push("inner");
		}
	} finally {
		log.push("outer");
	}
}

let finished = 0;

export function tupleThroughFinally(): LuaTuple<[number, string]> {
	try {
		return $tuple(1, "one");
	} finally {
		finished += 1;
	}
}

export function finishedCount() {
	return finished;
}

export function patternInCatch() {
	try {
		throw { code: 5 };
	} catch ({ code }: any) {
		return code as number;
	} finally {
		finished += 1;
	}
}
