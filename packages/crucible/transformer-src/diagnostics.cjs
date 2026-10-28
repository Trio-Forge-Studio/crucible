function createReporter(ts, addDiagnostic) {
	const seen = new Set();
	return (node, code, message) => {
		const key = `${node.getSourceFile().fileName}:${node.getStart()}:${code}`;
		if (seen.has(key)) return;
		seen.add(key);
		return addDiagnostic({
			category: ts.DiagnosticCategory.Error,
			code: 90000 + (/^CRU\d+$/.test(code) ? Number(code.slice(3)) : 0),
			source: "crucible",
			file: node.getSourceFile(),
			start: node.getStart(),
			length: node.getWidth(),
			messageText: `${code}: ${message}`,
		});
	};
}

module.exports = { createReporter };
