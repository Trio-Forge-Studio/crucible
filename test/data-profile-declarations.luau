local Engine = require("./lib/engine")
local World = require("./lib/profile-world")
local fixture = require("./lib/profile-fixture")
local spec = require("./lib/load")

local UNTRANSFORMED = "Crucible data needs the @trioforge/crucible/transformer plugin"

-- A module set with no server entry, as a client or a guard spec loads it. Any service lookup throws.
local function moduleSet()
	local g = spec.runtime({
		game = setmetatable({}, {
			__index = function(_, key)
				error(`a declaration touched game.{key}`)
			end,
		}),
	}, Engine.new().loggingPort)
	local emit = g.module("emit")
	return { graph = g, emit = emit, n = emit.n, api = g.module("") }
end

local function throws(fn, part: string, label: string)
	local ok, err = pcall(fn)
	assert(not ok, `{label}: expected a throw`)
	spec.has(err, part, label)
	return err
end

local function show(paths): string
	local out = {}
	for _, path in paths do
		table.insert(out, `\{{table.concat(path, ",")}\}`)
	end
	return table.concat(out, " ")
end

local function crucibleNames(r): { string }
	local names = {}
	for _, declaration in r.graph.module("internal/sync/declarations").declarations() do
		if string.sub(declaration.name, 1, 9) == "Crucible." then
			table.insert(names, declaration.name)
		end
	end
	return names
end

return {
	["1 untransformed profileStore and profileMessage throw the transformer message"] = function()
		local r = moduleSet()
		throws(function()
			r.api.profileStore("FD.Players", { template = {} })
		end, UNTRANSFORMED, "profileStore")
		local server = World.new({
			declare = function()
				return {}
			end,
		})
		throws(function()
			server.server.entry.profileMessage("FD.Gift")
		end, UNTRANSFORMED, "profileMessage")
	end,

	["4 129 a declaration touches no service, unwraps every mark into the defaults and ships the client template"] = function()
		local r = moduleSet()
		local store = fixture.declare(r)
		assert(type(store) == "table" and table.isfrozen(store), "a frozen callable table")
		spec.eq(next(store), nil, "with no fields of its own")
		local declaration = r.graph.module("internal/data/declarations").profileDeclarations()[1]
		local defaults = declaration.schema.defaults
		spec.eq(defaults.notes, "", "a marked scalar is unwrapped")
		spec.eq(defaults.vault.pin, 0, "a nested mark is unwrapped")
		spec.eq(defaults.analytics.playTime, 0, "a marked object is unwrapped")
		assert(table.isfrozen(defaults) and table.isfrozen(defaults.vault), "the defaults are deeply frozen")
		local client = declaration.clientTemplate
		spec.eq(client.notes, nil, "the client template drops serverOnly fields")
		spec.eq(client.vault, nil, "and the object left with no field")
		spec.eq(client.settings.volume, 0.5, "and keeps public defaults")
		assert(table.isfrozen(client), "frozen")
	end,

	["4 129 H1 E22 a template whose marks disagree with the emitted serverOnly paths throws at evaluation"] = function()
		local r = moduleSet()
		local persist, replica = fixture.nodes(r.n)
		local template = fixture.template(r.api)
		template.notes = ""
		throws(
			function()
				r.emit.profileStore("FD.Players", { template = template }, persist, replica, fixture.SERVER_ONLY)
			end,
			'FD.Players { "notes" } is serverOnly in its type but its template value is not marked',
			"an unmarked value"
		)
		local marked = fixture.template(r.api)
		marked.xp = r.api.serverOnly(0)
		throws(
			function()
				r.emit.profileStore("FD.Players", { template = marked }, persist, replica, fixture.SERVER_ONLY)
			end,
			'FD.Players { "xp" } is marked serverOnly in its template value but not in its type',
			"a mark the type lacks"
		)
		local n = r.n
		local nested = n.object({ { "n", n.object({ { "a", n.object({ { "b", n.f64 } }) }, { "a.b", n.f64 } }) } })
		throws(
			function()
				r.emit.profileStore("Game.Collide", {
					template = { n = { a = { b = r.api.serverOnly(1) }, ["a.b"] = 2 } },
				}, nested, n.object({ { "n", n.object({ { "a", n.object({}) } }) } }), { { "n", "a.b" } })
			end,
			'{ "n", "a", "b" } is marked serverOnly in its template value but not in its type',
			"a literal a.b key never matches a nested a.b"
		)
		spec.list(crucibleNames(r), {}, "no failed store declared a view")
	end,

	["131 replicatedPaths reads a declaration from another module set as segment arrays, escaped and sorted"] = function()
		local a = moduleSet()
		local store = fixture.declare(a)
		local b = moduleSet()
		spec.eq(
			show(b.api.replicatedPaths(store)),
			"{career} {career,jobs} {career,jobs,[],coins} {career,jobs,[],tip} {currency} {mastery} {mastery,*,level} {nickname} {questProgress} {settings} {settings,vibration} {settings,volume} {upgrades} {xp}",
			"FD.Players' replicated paths, without serverOnly fields or the emptied vault"
		)
		local n = a.n
		local keys = n.object({
			{ "*", n.f64 },
			{ "[]", n.f64 },
			{ "\\k", n.f64 },
			{ "a", n.object({ { "b", n.f64 } }) },
			{ "a.b", n.f64 },
		})
		local odd = a.emit.profileStore("Game.Keys", {
			template = { n = { ["*"] = 0, ["[]"] = 0, ["\\k"] = 0, a = { b = 0 }, ["a.b"] = 0 } },
		}, n.object({ { "n", keys } }), n.object({ { "n", keys } }), {})
		local paths = b.api.replicatedPaths(odd)
		spec.eq(
			show(paths),
			"{n} {n,\\*} {n,\\[]} {n,\\\\k} {n,a} {n,a,b} {n,a.b}",
			"a literal a.b key is one segment, marker-like keys are escaped, prefixes sort first"
		)
		spec.eq(type(paths[1]), "table", "a path is an array of segments, never a string")
		assert(table.isfrozen(paths) and table.isfrozen(paths[1]), "frozen")
		throws(function()
			b.api.replicatedPaths({})
		end, "replicatedPaths takes a profile store", "anything else")
	end,

	["129 M4 an object whose fields are all serverOnly has no declaration, path or client key, nested and at the root"] = function()
		local w = World.new({
			declare = function(r)
				local n = r.n
				local api = r.graph.module("")
				local hiddenPersist = n.object({
					{ "hidden", n.object({ { "a", n.f64 } }) },
					{ "mixed", n.object({ { "pub", n.f64 }, { "sec", n.f64 } }) },
				})
				return {
					Hidden = r.emit.profileStore(
						"Game.Hidden",
						{
							template = {
								hidden = { a = api.serverOnly(1) },
								mixed = { pub = 1, sec = api.serverOnly(2) },
							},
						},
						hiddenPersist,
						n.object({ { "mixed", n.object({ { "pub", n.f64 } }) } }),
						{
							{ "hidden", "a" },
							{ "mixed", "sec" },
						}
					),
					AllHidden = r.emit.profileStore(
						"Game.AllHidden",
						{
							template = { a = api.serverOnly(1), b = { c = api.serverOnly("x") } },
						},
						n.object({ { "a", n.f64 }, { "b", n.object({ { "c", n.string } }) } }),
						n.object({}),
						{
							{ "a" },
							{ "b", "c" },
						}
					),
				}
			end,
		})
		local ann = w:player("Ann", 4120)
		w:settle()
		w:settle()
		spec.list(crucibleNames(ann), {
			"Crucible.Data.Game.Hidden.mixed",
			"Crucible.Live.Game.AllHidden",
			"Crucible.Live.Game.Hidden",
		}, "no declaration for the hidden object or any AllHidden field")
		spec.eq(show(ann.api.replicatedPaths(ann.d.Hidden)), "{mixed} {mixed,pub}", "no path for the hidden object")
		spec.eq(show(ann.api.replicatedPaths(ann.d.AllHidden)), "", "no path at all for AllHidden")
		local root = ann.d.Hidden()()
		spec.eq(root.hidden, nil, "no client key for the hidden object")
		spec.eq(root.mixed.pub, 1, "the public field arrives")
		spec.eq(root.mixed.sec, nil, "the serverOnly field does not")
		spec.eq(ann.d.AllHidden:loaded(), true, "a store with no public field still loads on the client")
		local empty = ann.d.AllHidden()()
		spec.eq(next(empty), nil, "and reads an empty table")
		assert(table.isfrozen(empty), "frozen")
		local server = w.server.d.Hidden(ann.player)
		spec.eq(server.hidden.a(), 1, "the server reads the hidden object")
		spec.eq(server.hidden.a:set(5), true, "and writes it")
		w:finish()
	end,

	["2 3 names follow the grammar and length, are unique across profile and generic stores, and profile stores are capped"] = function()
		local r = moduleSet()
		local persist, replica = fixture.nodes(r.n)
		local function declareAs(name)
			return r.emit.profileStore(
				name,
				{ template = fixture.template(r.api) },
				persist,
				replica,
				fixture.SERVER_ONLY
			)
		end
		throws(function()
			declareAs("FD.1Players")
		end, "dot-separated segments", "a segment starting with a digit")
		throws(function()
			declareAs("Crucible.Players")
		end, "the name Crucible is reserved", "the reserved first segment")
		throws(function()
			declareAs(`FD.{string.rep("a", 48)}`)
		end, "a store name is 1 to 50 bytes", "51 bytes")
		declareAs("FD.Players")
		throws(function()
			declareAs("FD.Players")
		end, "FD.Players is already declared", "a duplicate profile store")
		throws(function()
			r.emit.dataStore("FD.Players", { defaults = {} }, r.n.object({}))
		end, "FD.Players is already declared", "a generic store with a profile store's name")
		r.emit.dataStore("FD.Log", { defaults = {} }, r.n.object({}))
		local small = r.n.object({ { "a", r.n.f64 } })
		throws(function()
			r.emit.profileStore("FD.Log", { template = { a = 0 } }, small, small, {})
		end, "FD.Log is already declared", "a profile store with a generic store's name")
		for i = 2, 4 do
			r.emit.profileStore(`FD.S{i}`, { template = { a = 0 } }, small, small, {})
		end
		throws(function()
			r.emit.profileStore("FD.S5", { template = { a = 0 } }, small, small, {})
		end, "FD.S5: more than 4 profile stores", "a fifth profile store")
	end,

	["2 134 A10 a field whose view name passes 64 bytes throws naming the field and the store's budget"] = function()
		local r = moduleSet()
		local store = `X.{string.rep("s", 33)}`
		local okField = string.rep("f", 64 - 15 - #store)
		local node = r.n.object({ { okField, r.n.f64 } })
		r.emit.profileStore(store, { template = { [okField] = 0 } }, node, node, {})
		local long = okField .. "g"
		local other = `Y.{string.rep("s", 33)}`
		local longNode = r.n.object({ { long, r.n.f64 } })
		throws(
			function()
				r.emit.profileStore(other, { template = { [long] = 0 } }, longNode, longNode, {})
			end,
			`{other}.{long}: its view name Crucible.Data.{other}.{long} is 65 bytes, over 64; this store allows field names of at most 14 bytes`,
			"65 bytes"
		)
		local hidden = r.n.object({ { long, r.n.f64 } })
		r.emit.profileStore(`Z.{string.rep("s", 33)}`, {
			template = { [long] = r.api.serverOnly(0) },
		}, hidden, r.n.object({}), { { long } })
	end,

	["3 a declaration after DataRuntime constructed throws and a failed boot unseals the registry"] = function()
		local w = World.new({
			declare = function(r)
				return { PlayerData = fixture.declare(r) }
			end,
		})
		local server = w.server
		local small = server.n.object({ { "a", server.n.f64 } })
		throws(function()
			server.emit.profileStore("FD.Late", { template = { a = 0 } }, small, small, {})
		end, "FD.Late was declared after the data registry sealed; declare stores in modules that boot loads", "late")
		local failed = World.new({
			declare = function(r)
				return { PlayerData = fixture.declare(r) }
			end,
			serve = function(s)
				s.service("Broken", function()
					error("constructor failed")
				end)
			end,
			mustBoot = false,
		})
		assert(not failed.booted, "boot failed")
		local fresh = failed.server.n.object({ { "a", failed.server.n.f64 } })
		local again = failed.server.emit.profileStore("FD.Again", { template = { a = 0 } }, fresh, fresh, {})
		assert(again ~= nil, "a failed boot left the data registry open")
	end,
}
