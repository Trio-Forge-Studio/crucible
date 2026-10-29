export async function retryStartup(launch, wait) {
	for (let attempt = 1; attempt <= 3; attempt++) {
		const result = await launch(attempt);
		if (!result.startupCrash || attempt === 3) return result.code;
		await wait(60000);
	}
}
