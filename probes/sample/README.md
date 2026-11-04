# Sample Studio probes

These drivers play the sample game's coins feature in Roblox Studio. Run them from the repository root on Windows with Studio signed in and Assistant MCP enabled, after `pnpm build`.

```powershell
node probes/sample/run.mjs r1
node probes/sample/live.mjs r1
node --test probes/sample/verification.test.mjs
```

`run.mjs` builds `examples/sample-game` with Rojo into `build/sample-probe/`, launches Studio on that place by PID, and starts a play test with one server and one client. The place name is unique, and the driver selects only the MCP session whose name matches it and whose process command line names the place. It prints a run marker from the server and from the client, presses `E` through the MCP keyboard tool, reads the HUD label on the client after each press, and saves the window with `probes/modules/window.ps1`. Studio runs the in-memory data and config ports, so the multiplier is 1 and the profile starts empty.

`live.mjs` opens the published test place 101113691364993 in universe 10774131158 with `-task EditPlace`, and never publishes. It proves its session is its own by printing a marker that must appear in the log of the PID it launched. The place holds no sample, so the driver parses a Rojo model of the sample and creates its instances through `execute_luau`, leaving out the package's own `node_modules`. It sets `CrucibleLiveData` on `ServerStorage`, so the real DataStore and ConfigService ports run. The place's `GlobalEvent` config is `{"multiplier":2,"endsAt":0}`, so each press pays 2. The coins persist in the experience's DataStore, so a later run starts where the last one stopped.

Each run writes `results/<label>.log` with the lines logged after the run marker, `-hud.json` with every HUD reading, `-session.json` with the PID, session ids and close method, `-console.json` and `-client.png`. Both drivers close Studio by its PID. Studio saves nothing: `live.mjs` stops the process, so the inserted sample stays in memory.
