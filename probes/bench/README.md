# Bench probes

These drivers measure what a change to the emitted Luau costs or saves at run time. Run them from the repository root on Windows with Studio signed in and Assistant MCP enabled, after `pnpm install` and a release build of the compiler.

```powershell
cargo build --release --locked --manifest-path Cargo.toml -p crucible-compiler --example compile
node probes/bench/run.mjs --reps 5
node --test probes/bench/verification.test.mjs
```

`examples/bench` is a place with six workloads in `src/shared/workloads`, one module each, and a server script that runs every workload once per Heartbeat. Each call runs inside a `debug.profilebegin("bench:<name>")` scope and is also timed with `os.clock`. Each workload returns a checksum that must match across calls and across variants.

| Workload | What it exercises |
|---|---|
| `loop` | C-style `for` loops over an array, with `size()` as the bound |
| `construct` | `new` on a small class and a method call on each object |
| `map` | `Array.map` with an arrow callback |
| `vector` | `Vector3` macros in a `for..of` loop. Both compilers emit nearly the same body, so it is the control |
| `pairs` | a whole `pairs()` step bound to one name, which allocates a table per step |
| `numeric` | arithmetic and `math.sqrt` in a C-style loop |

`run.mjs` builds three variants of the same source:

- `rbxtsc` is a recorded baseline. Its results in `results/rbxtsc-r*.json` came from roblox-ts 3.0.0 compiling `examples/bench` as a game at 838a4b2, before roblox-ts left the repository. `run.mjs` no longer builds it.
- `compiler` compiles it with `crucible-compiler`.
- `compiler-native` is the `compiler` output with `--!native` at the top of every workload module.

It downloads LibMP from the version and sha256 pinned in `libmp.json` into `build/libmp`, and adds it to each place as `ReplicatedStorage.LibMP`. LibMP never enters `examples/bench` itself. For each launch it waits until the machine's CPU load averages under 25 percent for ten seconds (`--quiet <percent>` changes it), builds a uniquely named place, starts Studio by PID, and starts play. After 300 warm-up frames it runs `capture.luau` on the Server datamodel, which snapshots 256 MicroProfiler frames into slot 1. It then reads the server script's own `os.clock` report and runs `summary.luau`, which reads each `bench:` scope's per-call time from the slot. Each rep runs every variant once, in an order that rotates by one each rep.

Each launch writes `results/<variant>-r<rep>.json`. The run writes `results/summary.json` and `results/summary.md`. The summary gives the median across launches of each launch's median per-call time, and the range across launches. A difference smaller than that range is not a measured difference.

## Reading the numbers

- Native code runs only in server scripts and on Android clients, so the workloads run on the server.
- The `KB per call` column is the heap change across a call from `gcinfo()`. When a garbage collection step runs inside a call the heap shrinks, so a workload whose 10th-percentile change is negative in any launch reads `GC inside calls` instead of a number. Rarer collections can still lower the median, and a positive change is a lower bound on what the call allocated.
- Studio numbers include the editor and the profiler. Compare them only with Studio runs on the same machine.
- A laptop lowers the clock of a busy core when other cores are busy. If `vector` differs between `rbxtsc` and `compiler`, the machine changed, not the code.
