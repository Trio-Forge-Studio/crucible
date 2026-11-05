# Random

Draw seeded random numbers that every server and client draws the same.

Roblox's `Random` makes no promise that a seed gives the same sequence on every server and client, or after an engine update. Roblox can change its algorithm and `Random:Shuffle` in any engine version, which matters as soon as a server and its clients need to agree on a draw. `Rng` runs [xoshiro128\*\*](https://prng.di.unimi.it/) in plain Luau instead, and its tests check the algorithm's reference output.

Import it from `@trioforge/crucible/util/random`. The subpath exports `Rng` and the types `SeedPart` and `RngState`. It has no dependencies, and the root, server and client entries never load it.

```ts
import { Rng } from "@trioforge/crucible/util/random";
import type { RngState, SeedPart } from "@trioforge/crucible/util/random";

const rng = new Rng("chalet", 12345);

print(rng.int(1, 6));
// â†’ 5
print(rng.pick(["lamp", "vase", "clock"]));
// â†’ vase
print(rng.shuffle([1, 2, 3, 4, 5]).join(" "));
// â†’ 5 4 1 2 3
```

## Seeds

`new Rng(...seed)` takes any mix of strings and integers (`SeedPart`) and hashes them into a 128-bit state. The same parts in the same order always give the same stream. Different parts give unrelated streams, so you can name a seed after what it is for:

```ts
import { Rng } from "@trioforge/crucible/util/random";

const houseSeed = 2024;
const furniture = new Rng("furniture", houseSeed);
const weather = new Rng("weather", houseSeed);
```

The two streams above are unrelated, and drawing from one never changes the other. Seed numbers must be integers within Â±2^53. `new Rng()` with no parts seeds itself from the clock and `math.random`. Use it for draws nobody needs to repeat.

`fork()` takes four draws from a stream and seeds a new one from them. Give a fork to a system that draws an unknown number of times, and the parent's sequence after the fork stays the same however many times the fork draws.

`state()` returns the four 32-bit words of the stream's position as an `RngState`. `Rng.fromState(state)` resumes from them, so a server can save a stream or send it to clients. `clone()` copies a stream at its current position.

## Draws

| Method | Returns |
| --- | --- |
| `uint32()` | An integer from 0 to 2^32 - 1, the generator's raw output |
| `float()` | A number in [0, 1) with 53 random bits |
| `float(min, max)` | A number in [min, max) |
| `int(min, max)` | An integer in [min, max], both inclusive, without modulo bias |
| `chance(probability)` | `true` with the given probability |
| `normal(mean?, deviation?)` | A normally distributed number, by default with mean 0 and deviation 1 |
| `pick(items)` | One item |
| `weightedIndex(weights)` | An index, chosen in proportion to its weight |
| `weighted(items, weight)` | An item, chosen in proportion to `weight(item, index)` |
| `shuffle(items)` | A shuffled copy. The array you pass stays as it was |
| `sample(items, count)` | `count` distinct items, in the order they were drawn |

Indices are 0-based. Weights must be finite and non-negative, with a positive total. A zero weight is never chosen.

```ts
import { Rng } from "@trioforge/crucible/util/random";

const rng = new Rng("loot", 7);
const rarities = ["common", "rare", "epic"];
print(rarities[rng.weightedIndex([80, 18, 2])]);
// â†’ common
```

Each method's sequence for a given state is part of the contract, so a change that would draw differently from the same seed is a breaking change.

## Moving from `@trioforge/random`

The subpath is the code of the retired `@trioforge/random` package, unchanged. Change the import to `@trioforge/crucible/util/random` and every seed draws the same numbers as before.
