function createReporter(ts, addDiagnostic) {
	return (node, code, message) =>
		addDiagnostic({
			category: ts.DiagnosticCategory.Error,
			code: Number(code.slice(3)),
			source: "crucible",
			file: node.getSourceFile(),
			start: node.getStart(),
			length: node.getWidth(),
			messageText: `${code}: ${message}`,
		});
}

module.exports = { createReporter };
