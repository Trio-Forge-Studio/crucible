-- An FD-shaped profile store, emitted by hand as the transformer would: the template, the persist descriptor, the
-- replica (ClientData: no serverOnly field and no object left with no field) and the serverOnly paths. Field names are
-- in byte order, as the transformer emits them.
local fixture = {}

fixture.STORE = "FD.Players"

-- The view declarations the replica owns (rule 134): the lifecycle and one per public top-level field.
fixture.LIVE = "Crucible.Live.FD.Players"
fixture.PUBLIC = { "career", "currency", "mastery", "nickname", "questProgress", "settings", "upgrades", "xp" }

function fixture.view(field: string): string
	return `Crucible.Data.FD.Players.{field}`
end

function fixture.nodes(n)
	local job = n.object({ { "coins", n.f64 }, { "tip", n.union({ n["nil"], n.string }) } })
	local career = n.object({ { "jobs", n.array(job) } })
	local mastery = n.map(n.string, n.object({ { "level", n.f64 } }))
	local settings = n.object({ { "vibration", n.bool }, { "volume", n.f64 } })
	local persist = n.object({
		{ "analytics", n.object({ { "playTime", n.f64 } }) },
		{ "career", career },
		{ "currency", n.f64 },
		{ "mastery", mastery },
		{ "nickname", n.union({ n["nil"], n.string }) },
		{ "notes", n.string },
		{ "purchaseLog", n.map(n.string, n.f64) },
		{ "questProgress", n.array(n.f64) },
		{ "settings", settings },
		{ "upgrades", n.map(n.string, n.f64) },
		{ "vault", n.object({ { "pin", n.f64 } }) },
		{ "xp", n.f64 },
	})
	local replica = n.object({
		{ "career", career },
		{ "currency", n.f64 },
		{ "mastery", mastery },
		{ "nickname", n.union({ n["nil"], n.string }) },
		{ "questProgress", n.array(n.f64) },
		{ "settings", settings },
		{ "upgrades", n.map(n.string, n.f64) },
		{ "xp", n.f64 },
	})
	return persist, replica
end

fixture.SERVER_ONLY = { { "analytics" }, { "notes" }, { "purchaseLog" }, { "vault", "pin" } }

-- The template as game code writes it, `serverOnly` marks included. Fresh tables on every call.
function fixture.template(api)
	return {
		analytics = api.serverOnly({ playTime = 0 }),
		career = { jobs = {} },
		currency = 0,
		mastery = {},
		notes = api.serverOnly(""),
		purchaseLog = api.serverOnly({}),
		questProgress = {},
		settings = { vibration = true, volume = 0.5 },
		upgrades = {},
		vault = { pin = api.serverOnly(0) },
		xp = 0,
	}
end

-- Declares FD.Players in realm `r` through the emit form. `options.template` and `options.migrations` override.
function fixture.declare(r, options)
	options = options or {}
	local api = r.graph.module("")
	local persist, replica = fixture.nodes(r.n)
	return r.emit.profileStore(fixture.STORE, {
		template = options.template or fixture.template(api),
		migrations = options.migrations,
	}, persist, replica, options.serverOnly or fixture.SERVER_ONLY)
end

return fixture
