# Creator Hub documentation record

Retrieved 2026-10-31. This is documented, not measured.
[ConfigService API](https://create.roblox.com/docs/reference/engine/classes/ConfigService) describes snapshots and testing overrides. It states no numeric request quota or complete allowed-character key grammar.
[Experience configs](https://create.roblox.com/docs/production/configs) lists 1,000 active configs and 100,000 characters for string or JSON values. It states no numeric request quota or complete key grammar.
[Cloud configs](https://create.roblox.com/docs/cloud/guides/configs) lists 100 keys per repository and a maximum key length of 256 characters. It states no complete allowed-character grammar.
The Cloud guide describes management endpoints, not in-engine GetConfigAsync request budgets.
No in-engine quota follows from those management limits.
DATA rule 102 already says ConfigService quotas are undocumented. The examined pages do not provide a number to replace that assumption.
Dotted-key runtime evidence is in deferred-short.log, not these documentation pages.
