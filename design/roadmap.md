# Crucible architecture roadmap

The plan in `architecture.md` ships as 11 pull requests in four milestones, plus one deletion-only pull request. Each pull request tells one story, proves itself with the same generated table, and marks which commits you read and which you skim.

## How review works

### One shape for every pull request

Every body has the same five parts, in this order.

1. **Story.** Two sentences on what changes and why.
2. **Proof.** A table that `cargo xtask proof` prints, which the agent pastes unedited. It shows:
   - whether the golden manifest is identical, and the changed files when it is not;
   - the arch violation count before and after;
   - the ABI check;
   - CI status;
   - each commit's kind.
3. **Read.** The commits and files you need to read, in reading order. The budget is about 800 lines of hand-written logic.
4. **Skim.** The mechanical commits, each with the evidence that it changes no logic.
5. **Not in this PR.** What a reader might expect here and which pull request has it.

### Two kinds of commit

- **Mechanical** commits move, rename, update paths or regenerate files. They carry a `Review: mechanical` trailer. `cargo xtask proof` checks that their renames keep at least 95% similarity and that every other edit changes only paths, and it compares the golden manifest at the pull request's head. You skim them.
- **Logic** commits carry `Review: read`. You read them.

Each commit passes `pnpm check` on its own, so the GitHub commit view works as a reading list. Squash merge keeps `main` at one commit per pull request.

### Rules that keep the count low

- One pull request is open for review at a time. At most one draft is stacked behind it.
- A pull request over budget splits at a commit boundary into a stacked follow-up. Splitting never scatters one story across unrelated pull requests.
- Before a pull request leaves draft, two adversarial reviewers (Opus and GPT) try to break it against the real tree. The agent fixes what they prove. Anything left open goes in the body.

### Where the work waits on you

The work runs on its own and stops only where it needs you.

| When | What you do |
| --- | --- |
| Each PR | Review and merge it. The next PR waits as a draft until you do. |
| After PR 1 | Approve the repository setting that makes the release workflow require CI. |
| Before PR 7's live tests | Provide a staging universe and an Open Cloud API key through the secret prompt. |
| After PR 9 | Authorize the 0.5.0 release. |

The agent makes the remaining choices itself, from objective scores, and records each one in the PR it affects. That covers the `Extension` trait bakeoff and the wording of the TLA+ invariants. You can overturn any of them in review.

## Which machine runs what

| Machine | Work |
| --- | --- |
| VPS (`hezaerd-vps`, Linux) | Rust compilation: PRs 1 and 3 to 8, PR 11, and the bakeoff prototypes. It matches CI's Linux job, which runs the Lune execution tests. At most two Cargo jobs run there at once, because tsrs is memory-hungry. |
| Laptop (`laptop-hezaerd`, Windows) | Coordination, PR 2's path move (CI runs on Windows too), TLA+ models (TLC on Java), the Windows check of every PR before it leaves draft, and Rust work that also needs Studio. |
| Mac (`MacBook Air de Yanne`) | Studio play tests of PRs 9 and 10, using the Release workflow artifacts of the PR. No Rust toolchain. |

## Milestone A. Foundation

No output byte changes and no user-visible change.

### PR 1. `chore(repo): add the refactor guardrails`

The goal is that every later pull request can prove itself.

- `cargo xtask golden` compiles the runtime in package mode and the sample game, and FD when a local checkout path is given. It compares an output hash manifest with a baseline recorded at `main`.
- `cargo xtask arch` enforces the five rules in `architecture.md`, with today's violation list checked in. The list may only shrink.
- `cargo xtask proof` prints the review table.
- CI runs the ignored JSX and network execution tests.
- The compiler's test-only fact exports move behind a `test-support` feature.
- AGENTS.md describes the repository as it is.

**Read:** the three xtask commands and the violation list.

### PR 2. `refactor(repo): move the Cargo workspace to the root`

- Commit 1 renames `cli/` to `crates/` and `cli/Cargo.toml` to the root `Cargo.toml`. It is 100% renames.
- Commit 2 updates paths in CI, npm scripts, tests, AGENTS.md and the release workflow.

**Read:** commit 2 only. `git show --stat -M` on commit 1 should list renames and nothing else.

### PR 3. `refactor(cli): extract the library crates`

These commits run in this order:
1. Extract `crucible-luau`.
2. Move `Safety`, `RetryPolicy`, `Auth`, `Payload` and the retry fields of `Op` out of `oc-registry`. This removes the cycle the extraction would create.
3. Extract `crucible-oc`, with an injected event sink, clock and sleeper.
4. Extract `crucible-project`.
5. Extract `crucible-docs`, holding topics and search only.
6. Replace the Mermaid `build.rs` panic with a `crucible-docs` test.

**Read:** the commits that inject the sink, clock and sleeper, and the Mermaid change. The extractions are mechanical.

## Milestone B. Compiler cut

The compiler stops naming the framework. Output stays identical until the last commit of PR 6.

**Before PR 4:** two or three `Extension` trait prototypes run in separate worktrees and are never opened as pull requests. Each is scored on four things: a golden manifest that stays identical, the arch violation count, FD build time against the 3 s budget, and the size of the trait. The agent picks the best score and puts the comparison in PR 4's body.

### PR 4. `refactor(compiler): add the extension seam and the ABI crate`

- The chosen `Extension` trait, wired in with no extension moved yet.
- `crucible-abi`, the generated `emit/abi.d.ts`, `cargo xtask abi check`, and the `crucibleAbi` handshake.
- The helper registry that replaces the `uses_crucible` and `uses_ui` flags.

**Read:** the trait, the ABI table and the handshake diagnostic.

### PR 5. `refactor(compiler): move the framework behind the seam`

There is one commit per feature, and each one shrinks the arch count:
1. Registration and DI.
2. Declarations and schemas.
3. Boot.
4. UI JSX and utility classes.
5. Typed lints. CRU032 to CRU034 move into the compiler, and CRU035 to CRU037 stay in the transform.
6. Rojo parsing moves to `crucible-project`, and the compiler takes an instance map.
7. Virtual modules in the compiler.

When the pull request is done, arch rule 1 has zero violations.

**Read:** the moved logic is mechanical. Read the glue each move adds, plus commits 6 and 7.

### PR 6. `refactor(compiler): make compiler and build state explicit`

- Compiler pass typestate and a validated `Facts` type.
- Build writes through temp files and renames.
- The outcome `Acceptance` enum.
- The last commit drops the TypeScript 5 union ordering. It is labeled as an output change, and its proof table lists every changed output file. It is the only commit in milestones A and B that changes bytes.

**Read:** all of it, and the golden diff of the last commit.

## Milestone C. Infrastructure as code

This milestone ends with the 0.5.0 release.

### PR 7. `feat(iac): model the experience and plan offline`

- The TLA+ provisioning model is at the top of the pull request. Its invariants are stated in plain English, so you can check them in review:
  - At most one remote resource exists per key per environment.
  - The lock never names a resource that doesn't exist.
  - No create follows an unknown outcome without an adopt check.
- The config model, strict TOML parsing with the lenient minimum-version pass, the JSON Schema, and offline `crucible plan`.
- Typed Open Cloud operations, with live tests against your staging universe. The live tests are skipped unless the key is present.

Nothing in this pull request writes to Roblox outside the gated tests.

**Read:** the invariants first, then the config model and the plan output.

### PR 8. `feat(iac): provision with a lock and a journal`

- `crucible.lock`, the `.crucible/journal/<env>.json` intent journal, and `crucible provision` with adopt-before-create.
- The `proptest-state-machine` conformance test, which runs against a fake Open Cloud.
- `cargo-mutants` on `crucible-iac` and `crucible-oc`.

**Read:** provision's state machine and the conformance test. The test is the evidence that the code matches the model.

### PR 9. `feat(iac): typed handles and deploy`

- Codegen of `src/crucible-experience.d.ts`, plus the per-environment bindings delivered through `crucible:experience`.
- Framework handle types, the handle overloads of `grant`, badge awards and pass checks, and the rule that every product has exactly one grant.
- `crucible deploy` replaces `crucible publish`.
- Docs for every new command.

**Read:** the handle types, the one-grant rule and deploy's preflight checks.

After this, you release 0.5.0 and FD adopts it in its own repository.

## Milestone D. Hardening

This milestone ships in 0.5.x patches and 0.6.

### PR 10. `refactor(runtime): encode the modeled protocols`

TLA+ models of profile authority with receipts, then of asks, with invariants stated in plain English at the top of the PR. Each modeled state becomes a discriminated union. The existing Lune specs and mutation campaigns must kill the same mutants as before. If both protocols together exceed the budget, the asks commits split into a stacked follow-up.

### PR 11. `test(compiler): run generated programs in Node and Lune`

The program generator, the two runners, the trace comparison, and a CI job with a fixed seed budget.

### Deletion PR. `refactor(cli): remove the roblox-ts migration`

This pull request ships in 0.6, one minor release after 0.5.0, per decision 7. It only deletes code.

## Not scheduled

The TypeScript config front end (`crucible.config.ts`) waits until 0.5.0 has been used. If you still want it after that, it gets its own milestone, starting with a prototype in a child process.

## If you flip a decision

| Decision | Effect on this roadmap |
| --- | --- |
| 1. TypeScript config first | Milestone C starts with the config prototype, and PR 7 grows. |
| 2. Delete the generic `crucible oc` | PR 1 gains a deletion-only commit removing about 3,400 lines. PR 3 loses the registry work. |
| 3. No move to `crates/` | PR 2 disappears. |
| 4. Public plugin API | A new milestone after D. The trait stays internal until then. |
| 5. CI provisions production | PR 8 gains a workflow that opens a lock pull request. It needs a bot token. |
| 6. Independent versions | PR 4's handshake carries the compatibility check, and the release script changes. |
| 7. Delete the migration now | The deletion PR moves into milestone A. |
| 8. Keep the TypeScript 5 ordering | PR 6 loses its last commit. |
