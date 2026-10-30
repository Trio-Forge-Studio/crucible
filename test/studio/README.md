# Sample Studio evidence

Run `pnpm build`, then `node test/studio/run.mjs Deferred short` and `node test/studio/run.mjs Immediate short`. The driver creates a unique place, selects its exact MCP name, checks its PID command line, and closes only that PID. PID reuse does not confer ownership. `run.mjs` retries only startup process exits, waits 60 seconds between attempts, and stops after three launches. Each crash session and available native log is archived before retry.

Both committed runs used Studio 0.745.0.7451137. Each session record names its launched PID and confirms `processRemaining=false`. No startup retry was needed. These runs execute the logger on both sides before boot.

The native logs record these observations in both signal modes:

- `[Sample.Server]` and `[Sample.Client]` emit Debug, Info and Warning through native output.
- Each side emits one red Error with its `TS.main` caller traceback and the spawned-error stack.
- The unchanged Crucible Renderer frame fault retains the observer, CrucibleObject, boot, scheduler and runner traceback frames.
- Tag removal, parent removal and re-admission preserve the binding lifecycle.
- Five render intervals record `Bind200,Crucible201,PreRender`.
- Controller shutdown completes before Store shutdown, within the existing 25-second bound.

`results/deferred-short.log` and `results/immediate-short.log` contain the filtered traces. The matching `-native.log` files retain all original error lines. Their `-session.json` files record the owned process, Studio version and launch and shutdown times. Native logs preserve line order and contents except line-ending normalization and trailing whitespace removal.

`observations.mjs` checks the compiled sample against the actual Roblox port. Its client observer requires the StarterPlayer template discovered by boot. Requiring the PlayerScripts clone would execute a second registration. The observer wraps the sample renderer for five frames, then replaces its frame hook with a deliberate error.

## Lifecycle measurements retained by the driver

`measurements.mjs` imports cloned compiled lifecycle modules through Roblox's roblox-ts RuntimeLib. Each case has isolated module state. Pool effects use the real DenseList for membership. No fake engine executes these measurements. Five trials alternate pool, batches and chained requests. Each trial verifies the expected membership and cap counts. Instrumented trials retain a time breakdown separately from the five trials. These values are raw observations from the existing driver. This logger change makes no performance comparison claim.

Shutdown elapsed time is measured from the driver's Stop request to each hook's native-log timestamp. This includes Studio orchestration and is an upper bound on the sample hook path, not hook CPU cost.

## Regression and mutation checks

`node --test test/studio/verification.test.mjs` checks native logger severity, game caller tracebacks, the existing Crucible fault, owned-process identity, exact session selection, ambiguity rejection, startup retries, render order and shutdown evidence in both modes.

`node test/lib/logging-mutants.mjs` rebuilds source mutants in an isolated scratch directory and runs the named Lune spec for each. `test/logging-mutants.log` records build exit 0 and spec exit 1 for each killed mutant. The runner removes scratch with Node's `fs.rmSync`.

`node test/sample-mutants.mjs` covers fingerprint invalidation, real fixture copying, PID ownership, retry waiting and sample wiring. Its existing receipt is `results/sample-mutants.log`.

The Lune sample fixture includes a tagged Part without required Identity. The compiled lifecycle spec rejects its admission. Its valid Part has Identity fleet-23 and absent Speed, so field substitution and fallback errors remain distinguishable.
