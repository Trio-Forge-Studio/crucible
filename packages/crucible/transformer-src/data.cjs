function createDataChecks({ ts, program, checker, api }) {
	const unwrap = (expression) => {
		while (ts.isParenthesizedExpression(expression) || ts.isNonNullExpression(expression))
			expression = expression.expression;
		return expression;
	};
	const member = (expression) => {
		if (ts.isPropertyAccessExpression(expression)) return expression.name.text;
		if (ts.isElementAccessExpression(expression) && ts.isStringLiteralLike(expression.argumentExpression))
			return expression.argumentExpression.text;
	};
	const store = (expression) => api(checker.getTypeAtLocation(expression).getSymbol()) === "ProfileStore";
	const marketplace = (expression) => {
		const symbol = checker.getTypeAtLocation(expression).getSymbol();
		return (
			symbol?.name === "MarketplaceService" &&
			symbol.declarations?.some((d) =>
				d.getSourceFile().fileName.replaceAll("\\", "/").includes("/@rbxts/types/"),
			)
		);
	};
	const storeCall = (node) => {
		const callee = unwrap(node.expression);
		if (store(callee)) return { store: callee };
		const name = member(callee);
		if (name !== undefined && store(callee.expression)) return { store: callee.expression, member: name };
	};
	let granted;
	const grants = () => {
		if (granted === undefined) {
			granted = false;
			const walk = (node) => {
				if (granted) return;
				if (ts.isCallExpression(node) && storeCall(node)?.member === "grant") granted = true;
				else ts.forEachChild(node, walk);
			};
			for (const source of program.getSourceFiles()) if (!source.isDeclarationFile) walk(source);
		}
		return granted;
	};
	return ({ report, side }) =>
		(node) => {
			if (ts.isCallExpression(node)) {
				const call = storeCall(node);
				if (call) {
					const text = call.store.getText();
					const client =
						call.member === undefined || call.member === "loaded" ? !node.arguments.length : false;
					const required = client ? "client" : "server";
					if (side(node) !== required)
						report(
							node,
							"CRU027",
							client
								? `${text}${call.member ? ".loaded" : ""}() reads the local player's profile; on the server call ${text}${call.member ? ".loaded" : ""}(player).`
								: `${text}${call.member === undefined ? "(player)" : `.${call.member}`} is server-only; the client reads ${text}().`,
						);
				}
			}
			if (ts.isBinaryExpression(node) && node.operatorToken.kind === ts.SyntaxKind.EqualsToken) {
				const target = unwrap(node.left);
				if (member(target) === "ProcessReceipt" && marketplace(target.expression) && grants())
					report(
						node,
						"CRU025",
						"Crucible's data module owns ProcessReceipt; register grants with a profile store's grant(productId, ...).",
					);
			}
		};
}

module.exports = { createDataChecks };
