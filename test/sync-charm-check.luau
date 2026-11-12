local spec = require("./lib/load")
local Link = require("./lib/network-link")

return {
	["side entries without replicated state run no Charm Sync probe"] = function()
		local graph = Link.new().server.graph
		local patch = graph.package("@rbxts/charm-sync/src").patch
		local calls = 0
		local diff = patch.diff
		patch.diff = function(...)
			calls += 1
			return diff(...)
		end
		graph.module("server")
		spec.eq(calls, 0, "no replicated state, no probe")
		graph.module("emit").replicated("Check.First", 0, graph.module("emit").n.f64)
		assert(calls > 0, "first declaration ran the probe")
		local before = calls
		graph.module("emit").replicated("Check.Second", 0, graph.module("emit").n.f64)
		spec.eq(calls, before, "subsequent declarations do not probe again")
	end,
	["pinned Charm Sync passes its load check"] = function()
		local graph = spec.runtime()
		graph.module("internal/sync/charm-sync").checkCharmSync()
		graph.module("internal/sync/charm-sync").checkCharmSync()
	end,
	["changed Charm Sync shapes fail when replicated declarations load"] = function()
		for _, change in
			{
				{ "apply = apply,", "applyImmutable = apply," },
				{ '__none = "__none"', '__none = "changed"' },
				{ "local newState = table.clone(oldState)", "local newState = oldState" },
				{ "local diffs = table.clone(newState)", "local diffs = newState" },
				{ "if next(diffs) == nil then", "if false then" },
			}
		do
			local changed = 0
			spec.rewrite(function(path, source)
				if string.sub(path, -#"charm-sync/src/patch") == "charm-sync/src/patch" then
					local start, finish = string.find(source, change[1], 1, true)
					assert(start, "mutation target exists")
					changed += 1
					return string.sub(source, 1, start - 1) .. change[2] .. string.sub(source, finish + 1)
				end
				return source
			end)
			local ok, err = pcall(function()
				local graph = Link.new().server.graph
				graph.module("server")
				local emit = graph.module("emit")
				emit.replicated("Check.Value", 0, graph.module("internal/network/schema").n.f64)
			end)
			spec.rewrite(nil)
			spec.eq(changed, 1, "changed the installed patch module")
			spec.eq(ok, false, "a changed copy fails")
			spec.has(err, "Crucible needs @rbxts/charm-sync 0.4.1", "expected version")
			spec.has(err, "installed version unavailable", "installed version metadata limitation")
			spec.has(err, "then rebuild the game", "fix")
		end
	end,
}
