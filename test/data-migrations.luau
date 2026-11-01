local spec = require("./lib/load")
local D2 = require("./lib/data-d2")
local DM = require("./lib/data-migrations")

local n = DM.n
local STORE = DM.STORE
local USER = DM.USER
local SECRET = "SECRET-payload-value"

local ROOT = DM.object({ coins = n.f64, gems = n.f64, label = DM.opt(n.string) })

-- Two steps whose order shows in the result: doubling then adding 5 gives 19 from 7, adding then doubling gives 24.
local function twoSteps(log)
	return {
		node = ROOT,
		migrations = {
			function(old)
				table.insert(log, { step = 0, coins = old.coins })
				return { coins = old.coins * 2 }
			end,
			function(old)
				table.insert(log, { step = 1, coins = old.coins })
				return { coins = old.coins + 5, label = "m2" }
			end,
		},
	}
end

local OLD = { coins = 7, gems = 3 }
local MIGRATED = { coins = 19, gems = 3, label = "m2" }

local function seedOld(backend, fields)
	local seed = { schemaVersion = 0, payload = OLD, epoch = 5 }
	for key, value in fields or {} do
		seed[key] = value
	end
	D2.seed(backend, nil, seed)
	return seed
end

local function steps(log)
	local order = {}
	for _, entry in log do
		table.insert(order, entry.step)
	end
	return order
end

local function untouchedSeed(payload)
	return { schemaVersion = 0, payload = payload, epoch = 5 }
end

-- A step shape whose step `failing` of two fails in `mode`.
local function failingShape(mode: string, failing: number, holder, ran)
	local function fail()
		if mode == "throws" then
			error("secret-boom")
		elseif mode == "yields" then
			coroutine.yield()
		elseif mode == "waits" then
			holder.realm.wait(1)
		elseif mode == "string" then
			return "not a record"
		elseif mode == "number" then
			return 42
		end
		return nil
	end
	local migrations = {}
	for index = 0, 1 do
		migrations[index + 1] = function(old)
			table.insert(ran, index)
			if index == failing then
				return fail()
			end
			return { coins = old.coins + 1 }
		end
	end
	return { migrations = migrations }
end

return {
	["rule 74 a version 0 record with two steps migrates, counts one revision and saves schemaVersion 2 with the payload"] = function()
		local log = {}
		local backend, a, b = D2.world(2, { version = twoSteps(log) })
		seedOld(backend)
		local pa, handle = DM.begin(a)
		backend:advance(0.5)
		spec.eq(handle.value, true, "loaded")
		local session = DM.session(a, pa)
		spec.eq(session.revision, 1, "the migration is revision 1")
		DM.deepEq(session.value, MIGRATED, "the live value is the migrated record")
		spec.list(steps(log), { 0, 1 }, "both steps ran once, in order")
		spec.eq(log[1].coins, 7, "step 0 saw the stored record")
		spec.eq(log[2].coins, 14, "step 1 saw step 0's merged result")
		local calls = backend:callsTo({ server = a.id })
		spec.eq(#calls, 2, "the acquisition and one save, already requested")
		assert(calls[2].at < 0.3, `the save was requested at once, not at the renewal (t = {calls[2].at})`)
		spec.eq(calls[2].committed, true, "the save committed")
		local envelope = DM.stored(backend)
		spec.eq(envelope.schemaVersion, 2, "version committed with the payload")
		DM.deepEq(envelope.payload, MIGRATED, "migrated payload")
		spec.eq(envelope.epoch, 6, "one acquisition")
		spec.eq(envelope.commit.revision, 1, "the migration's revision")
		spec.eq(envelope.commit.token, session.token, "under the session's token")
		spec.eq(envelope.lease.serverId, a.id, "the lease is the session's")
		spec.eq(session.confirmed, 1, "confirmed")

		DM.leave(backend, a, pa)
		local pb = D2.join(backend, b)
		spec.eq(#log, 2, "a second realm of the same build runs no step")
		local second = DM.session(b, pb)
		DM.deepEq(second.value, MIGRATED, "the same value")
		spec.eq(second.revision, 0, "nothing migrated, so revision 0")
		spec.eq(#backend:callsTo({ server = b.id }), 1, "and no save is requested")
		spec.eq(DM.stored(backend).schemaVersion, 2, "still version 2")
	end,
	["rule 27 and 74 an ambiguous commit never stores the migrated payload under version 0 or the old one under 2"] = function()
		local log = {}
		local backend, a = D2.world(1, { version = twoSteps(log) })
		seedOld(backend)
		backend:fail("committed", { server = a.id }, 1, { after = 1 })
		local beforeCommit = backend:gate("commit", { index = 2 })
		local afterCommit = backend:gate("committed", { index = 2 })
		local pa, handle = DM.begin(a)
		backend:advance(0.5)
		spec.eq(handle.value, true, "loaded")
		spec.eq(backend.calls[2].parked, "commit", "the save is parked before its commit")
		local parked = DM.stored(backend)
		spec.eq(parked.schemaVersion, 0, "the stored version is still 0")
		DM.deepEq(parked.payload, OLD, "with the old payload")

		beforeCommit.release()
		spec.eq(backend.calls[2].parked, "committed", "committed, response held")
		local committed = DM.stored(backend)
		spec.eq(committed.schemaVersion, 2, "version 2 landed in the same write")
		DM.deepEq(committed.payload, MIGRATED, "as the migrated payload")

		afterCommit.release()
		for _ = 1, 120 do
			backend:advance(0.05)
			local envelope = DM.stored(backend)
			local pair = (envelope.schemaVersion == 0 and envelope.payload.coins == 7)
				or (envelope.schemaVersion == 2 and envelope.payload.coins == 19)
			assert(pair, `a torn pair: version {envelope.schemaVersion} with coins {envelope.payload.coins}`)
		end
		spec.eq(backend.calls[2].plan, "committed", "the first save committed and answered with an error")
		spec.eq(backend.calls[3].committed, nil, "the retry found its own commit and wrote nothing")
		spec.eq(#backend:callsTo({ server = a.id }), 3, "no other call yet")
		local final = DM.stored(backend)
		spec.eq(final.schemaVersion, 2, "ends at version 2")
		DM.deepEq(final.payload, MIGRATED, "with the migrated payload")
		spec.eq(DM.session(a, pa).confirmed, 1, "the retry confirmed what it found")
		spec.eq(#log, 2, "the steps ran once")
	end,
	["rule 74 a record at version v of N runs exactly migrations v to N-1, in order"] = function()
		local expected = {
			[0] = { order = { 0, 1, 2 }, value = 1012 },
			[1] = { order = { 1, 2 }, value = 112 },
			[2] = { order = { 2 }, value = 12 },
			[3] = { order = {}, value = 1 },
		}
		for version = 0, 3 do
			local order = {}
			local migrations = {}
			for index = 0, 2 do
				migrations[index + 1] = function(old)
					table.insert(order, index)
					return { n = old.n * 10 + index }
				end
			end
			local backend, a = D2.world(1, { version = { migrations = migrations } })
			D2.seed(backend, nil, { schemaVersion = version, payload = { n = 1 }, epoch = 2 })
			local pa, handle = DM.load(backend, a)
			spec.eq(handle.value, true, `v{version}: loaded`)
			spec.list(order, expected[version].order, `v{version}: steps run`)
			local session = DM.session(a, pa)
			spec.eq(session.value.n, expected[version].value, `v{version}: value`)
			spec.eq(session.revision, if version < 3 then 1 else 0, `v{version}: revision`)
			spec.eq(DM.stored(backend).schemaVersion, 3, `v{version}: stored version`)
			spec.eq(DM.stored(backend).payload.n, expected[version].value, `v{version}: stored value`)
		end
	end,
	["rule 18 and 74 a key with no envelope, and one with only inbox entries, take the defaults at version N and run no step"] = function()
		local log = {}
		local backend, a = D2.world(1, { version = twoSteps(log) })
		local pa, handle = DM.begin(a)
		backend:advance(1)
		spec.eq(handle.value, true, "an empty key loads")
		local session = DM.session(a, pa)
		DM.deepEq(session.value, { coins = 0, gems = 0 }, "the defaults")
		spec.eq(session.revision, 0, "no revision")
		spec.eq(#log, 0, "no step ran")
		spec.eq(#backend:callsTo({ server = a.id }), 1, "and no save was requested")
		spec.eq(DM.stored(backend).payload, nil, "nothing stored yet")
		backend:advance(25)
		local renewed = DM.stored(backend)
		spec.eq(renewed.schemaVersion, 2, "the first renewal stores the current version")
		DM.deepEq(renewed.payload, { coins = 0, gems = 0 }, "with the defaults")

		local second, b = D2.world(1, { version = twoSteps(log) })
		local id = D2.id(D2.WALL, "m1")
		D2.seed(second, nil, { unopened = true, epoch = 3, inbox = { D2.item(id, "grant", { amount = 1 }) } })
		local pb, loaded = DM.begin(b)
		second:advance(1)
		spec.eq(loaded.value, true, "an inbox-only envelope loads")
		DM.deepEq(DM.session(b, pb).value, { coins = 0, gems = 0 }, "the defaults, with schemaVersion 0 stored below N")
		spec.eq(#log, 0, "no step ran for it either")
		spec.eq(DM.session(b, pb).revision, 0, "no revision")
		spec.eq(DM.stored(second).inbox[1].id, id, "its inbox entry is kept")
	end,
	["rule 74 a migration rerun after a crash reaches the same value and stores version N"] = function()
		local log = {}
		local backend, a, b = D2.world(2, { version = twoSteps(log) })
		seedOld(backend)
		backend:gate("commit", { index = 2 })
		local _, handle = DM.begin(a)
		backend:advance(0.5)
		spec.eq(handle.value, true, "A migrated in memory")
		spec.eq(#log, 2, "A ran both steps")
		a.crash()
		backend:advance(1)
		local stuck = DM.stored(backend)
		spec.eq(stuck.schemaVersion, 0, "the stored record is still the old version")
		DM.deepEq(stuck.payload, OLD, "with the old payload")
		local pb = D2.joinSlow(backend, b)
		backend:advance(1)
		spec.eq(#log, 4, "B reran both steps")
		spec.eq(log[3].step, 0, "from step 0")
		spec.eq(log[3].coins, 7, "on the same input")
		spec.eq(log[4].coins, 14, "and the same intermediate")
		DM.deepEq(DM.session(b, pb).value, MIGRATED, "the same value")
		local envelope = DM.stored(backend)
		spec.eq(envelope.schemaVersion, 2, "B's save stores version N")
		DM.deepEq(envelope.payload, MIGRATED, "with the migrated payload")
		spec.eq(envelope.epoch, 7, "two acquisitions since the seed's 5")
	end,
	["rule 49 inspect of an old-version record returns the migrated value and writes nothing"] = function()
		local log = {}
		local backend, a = D2.world(1, { version = twoSteps(log) })
		local seed = seedOld(backend)
		local before = backend:stored(STORE, tostring(USER))
		local result = DM.inspect(backend, a)
		spec.eq(result.kind, "ok", "readable")
		DM.deepEq(result.value, MIGRATED, "the migrated, admitted value")
		spec.eq(table.isfrozen(result.value), true, "frozen")
		spec.eq(#log, 2, "the steps ran in memory")
		local calls = backend:callsTo({ server = a.id })
		spec.eq(#calls, 1, "one call")
		spec.eq(calls[1].kind, "read", "a GetAsync")
		spec.eq(calls[1].committed, nil, "nothing committed")
		DM.deepEq(
			backend:stored(STORE, tostring(USER)),
			before,
			"the stored entry, version counter included, is unchanged"
		)
		spec.eq(DM.stored(backend).epoch, seed.epoch, "no epoch step")
		spec.eq(DM.stored(backend).lease, nil, "inspect opened nothing")
	end,
	["rule 75 a step gets a private copy and the next step sees the merged state, not its sibling's edits"] = function()
		local seen = {}
		local migrations = {
			function(old)
				old.coins = 999
				old.nested.x = 1
				old.added = true
				return {}
			end,
			function(old)
				table.insert(seen, { coins = old.coins, x = old.nested.x, added = old.added })
				return {}
			end,
		}
		local backend, a = D2.world(1, { version = { migrations = migrations } })
		D2.seed(backend, nil, { payload = { coins = 7, nested = { x = 0 } } })
		local pa = D2.join(backend, a)
		local session = DM.session(a, pa)
		spec.eq(session.value.coins, 7, "the loaded value is unchanged")
		spec.eq(session.value.nested.x, 0, "also nested")
		spec.eq(session.value.added, nil, "and no key was added")
		spec.eq(seen[1].coins, 7, "step 1 saw the merged state")
		spec.eq(seen[1].x, 0, "nested too")
		spec.eq(seen[1].added, nil, "without step 0's edits")
		backend:advance(1)
		DM.deepEq(DM.stored(backend).payload, { coins = 7, nested = { x = 0 } }, "and so is the stored record")
		spec.eq(DM.stored(backend).schemaVersion, 2, "the empty patches only marked the version")
	end,
	["rule 75 a patch merges objects, replaces everything else verbatim and keeps omitted keys"] = function()
		local migrations = {
			function(old)
				return {
					coins = 6,
					renamed = old.gone,
					profile = { stats = { hp = 9 }, tags = { "q" }, scalar = { deep = true } },
					list = {},
				}
			end,
			function()
				return { profile = { stats = 5 }, keep = {} }
			end,
		}
		local backend, a = D2.world(1, { version = { migrations = migrations } })
		D2.seed(backend, nil, {
			payload = {
				coins = 5,
				keep = "kept",
				gone = 1,
				profile = { name = "a", tags = { "x", "y", "z" }, stats = { hp = 1, mp = 2 }, scalar = 4 },
				list = { 1, 2, 3 },
			},
		})
		local pa = D2.join(backend, a)
		DM.deepEq(DM.session(a, pa).value, {
			coins = 6,
			keep = {},
			gone = 1,
			renamed = 1,
			profile = { name = "a", tags = { "q" }, stats = 5, scalar = { deep = true } },
			list = {},
		}, "the merged record")
		backend:advance(1)
		spec.eq(DM.stored(backend).payload.gone, 1, "a step cannot delete a key: the old name of a rename stays")
		spec.eq(DM.stored(backend).payload.renamed, 1, "the new name is written")
		spec.eq(DM.stored(backend).payload.profile.name, "a", "an omitted nested key is kept")
		spec.list(DM.stored(backend).payload.profile.tags, { "q" }, "the array is replaced, not merged")
	end,
	["rule 75 an empty patch only marks the version"] = function()
		local backend, a = D2.world(1, { version = { migrations = {
			function()
				return {}
			end,
		} } })
		D2.seed(backend, nil, { payload = { coins = 4, extra = { 1, 2 } } })
		local pa = D2.join(backend, a)
		spec.eq(DM.session(a, pa).revision, 1, "still a revision")
		backend:advance(1)
		local envelope = DM.stored(backend)
		spec.eq(envelope.schemaVersion, 1, "the version moved")
		DM.deepEq(envelope.payload, { coins = 4, extra = { 1, 2 } }, "the payload did not")
	end,
	["rule 75 a step that throws, yields or returns a non-record fails the load at once and leaves the data untouched"] = function()
		local payload = { coins = 7, gems = 3, secret = SECRET }
		for _, mode in { "throws", "yields", "waits", "string", "number", "nil" } do
			for failing = 0, 1 do
				local label = `{mode} at step {failing}`
				local holder, ran = {}, {}
				local backend, a = D2.world(1, { version = failingShape(mode, failing, holder, ran) })
				holder.realm = a
				D2.seed(backend, nil, untouchedSeed(payload))
				local pa, handle = DM.begin(a)
				backend:advance(1)
				DM.assertRefused(backend, a, pa, handle, a.limits.LOAD_FAILED_MESSAGE, untouchedSeed(payload), label)
				spec.list(ran, if failing == 0 then { 0 } else { 0, 1 }, `{label}: steps that ran`)
				local messages = DM.logged(a, "migrations[")
				spec.eq(#messages, 1, `{label}: one diagnostic`)
				spec.has(messages[1], `migrations[{failing}]`, `{label}: names the failing step`)
				spec.has(messages[1], STORE, `{label}: names the store`)
				spec.has(messages[1], tostring(USER), `{label}: names the user`)
				spec.eq(a.engine.logged(`migrations[{1 - failing}]`), 0, `{label}: and not the other step`)
				spec.eq(a.engine.logged(SECRET), 0, `{label}: no data value in any diagnostic`)
				spec.eq(a.engine.logged("secret-boom"), 0, `{label}: nor the step's error text`)
			end
		end
	end,
	["rule 110 a failed migration is diagnosed once for a user who rejoins, with the store, user and step"] = function()
		local holder, ran = {}, {}
		local backend, a = D2.world(1, { version = failingShape("throws", 1, holder, ran) })
		D2.seed(backend, nil, untouchedSeed({ coins = 7, secret = SECRET }))
		local first, handle = DM.begin(a)
		backend:advance(1)
		spec.eq(handle.value, false, "refused")
		a.leave(first)
		for _ = 1, 2 do
			local again, retry = DM.begin(a)
			backend:advance(1)
			spec.eq(retry.value, false, "refused again")
			spec.eq(a.kicked(again), a.limits.LOAD_FAILED_MESSAGE, "kicked again")
			a.leave(again)
		end
		spec.eq(#ran, 6, "the step ran on every attempt")
		local messages = DM.logged(a, "migrations[1]")
		spec.eq(#messages, 1, "one diagnostic for three attempts")
		spec.has(messages[1], `{STORE}`, "the store")
		spec.has(messages[1], `{USER}`, "the user")
		spec.eq(a.engine.logged(SECRET), 0, "no value")
	end,
	["rule 110 a failed migration is diagnosed once per store and code, not once per user"] = function()
		local holder, ran = {}, {}
		local backend, a = D2.world(1, { version = failingShape("throws", 1, holder, ran) })
		for _, user in { 4120, 4121, 4122 } do
			D2.seed(backend, user, untouchedSeed({ coins = 7, secret = SECRET }))
			local player, handle = DM.begin(a, user)
			backend:advance(1)
			spec.eq(handle.value, false, `refused {user}`)
			spec.eq(a.kicked(player), a.limits.LOAD_FAILED_MESSAGE, `kicked {user}`)
		end
		spec.eq(#DM.logged(a, "migrations[1]"), 1, "one diagnostic for three users")
		spec.eq(a.engine.logged(SECRET), 0, "no value")
	end,
	["rule 77 and durability sequence an older build meets a newer version"] = function()
		local fills = 0
		local function fillTips(old)
			fills += 1
			local jobs = {}
			for index, job in old.career.jobs do
				local copy = table.clone(job)
				copy.tips = 0
				jobs[index] = copy
			end
			return { career = { jobs = jobs } }
		end
		local function root(job)
			return DM.object({ career = DM.object({ jobs = n.array(job) }), coins = n.f64 })
		end
		local jobN1 = DM.object({ coins = n.f64, id = n.string, rating = DM.opt(n.f64) })
		local jobN2 = DM.object({ coins = n.f64, id = n.string, rating = DM.opt(n.f64), tips = n.f64 })
		local shapeN2 = { node = root(jobN2), migrations = { fillTips } }
		local shapeN1 = { node = root(jobN1), version = 0 }
		local defaults = { career = { jobs = {} }, coins = 0 }
		local backend, d, e, f = D2.world(3, { versions = { shapeN2, shapeN1, shapeN2 }, defaults = defaults })
		local record = {
			career = { jobs = { { id = "j1", coins = 10 }, { id = "j3", coins = 40, rating = 5 } } },
			coins = 100,
		}
		D2.seed(backend, nil, { schemaVersion = 0, epoch = 8, payload = record })

		local pd = DM.load(backend, d, nil, 0.5)
		spec.eq(fills, 1, "D ran fillTips once")
		local migrated = DM.stored(backend)
		spec.eq(migrated.schemaVersion, 1, "version 1 committed with the payload")
		spec.eq(migrated.epoch, 9, "epoch 9")
		spec.eq(migrated.lease.serverId, d.id, "lease D")
		local expectedJobs = { { id = "j1", coins = 10, tips = 0 }, { id = "j3", coins = 40, rating = 5, tips = 0 } }
		DM.deepEq(
			migrated.payload,
			{ career = { jobs = expectedJobs }, coins = 100 },
			"each element copied whole, rating kept"
		)

		backend:advance(300 - backend.now)
		DM.leave(backend, d, pd)
		local released = DM.stored(backend)
		spec.eq(released.lease, nil, "D released")
		spec.eq(released.schemaVersion, 1, "version 1")
		backend:advance(330 - backend.now)
		local before = backend:stored(STORE, tostring(USER))

		local pe, loading = DM.begin(e)
		backend:advance(1)
		spec.eq(loading.value, false, "E's load fails")
		local kick = DM.kick(e, pe)
		spec.eq(kick.message, e.limits.SERVER_UPDATING_MESSAGE, "kicked as updating")
		assert(kick.at - 330 < 0.5, `kicked at once (t = {kick.at})`)
		DM.deepEq(
			backend:stored(STORE, tostring(USER)),
			before,
			"the stored entry is unchanged: lease, epoch, version counter"
		)
		local calls = backend:callsTo({ server = e.id })
		spec.eq(#calls, 1, "E made one call: the acquisition")
		spec.eq(calls[1].committed, nil, "it wrote nothing")

		local inspected = DM.inspect(backend, e)
		spec.eq(inspected.kind, "unavailable", "inspect on E is unavailable")
		local resumed = e.run(function()
			return e.store:resume(pe)
		end)
		backend:advance(1)
		spec.eq(resumed.value, false, "resume on E does what a load does")
		local kicks = 0
		for _, entry in e.kicks do
			if entry.player == pe then
				kicks += 1
				spec.eq(entry.message, e.limits.SERVER_UPDATING_MESSAGE, "every kick is the updating message")
			end
		end
		spec.eq(kicks, 2, "one for the load and one for the resume")
		for _, call in backend:callsTo({ server = e.id }) do
			spec.eq(call.committed, nil, `E's call {call.index} ({call.kind}) wrote nothing`)
		end
		DM.deepEq(backend:stored(STORE, tostring(USER)), before, "still unchanged")

		local decl = D2.message("grant")
		local id = D2.id(D2.wallAt(backend), "m1")
		local sent = D2.send(e, decl, { amount = 1 }, id)
		backend:advance(1)
		spec.eq(sent.value.kind, "ok", "a send from E appends")
		local appended = DM.stored(backend)
		spec.eq(#appended.inbox, 1, "one item queued")
		spec.eq(appended.inbox[1].version, 0, "stamped with E's version")
		spec.eq(appended.schemaVersion, 1, "the stored version is untouched")
		spec.eq(appended.epoch, 9, "and the epoch")
		DM.deepEq(appended.payload, migrated.payload, "and the payload")

		backend:advance(345 - backend.now)
		local received = D2.receiver(f, decl)
		local pf, floaded = DM.begin(f)
		backend:advance(1)
		spec.eq(floaded.value, true, "F loads version 1 normally")
		spec.eq(fills, 1, "and runs no step")
		local after = DM.stored(backend)
		spec.eq(after.epoch, 10, "epoch 10")
		spec.eq(after.lease.serverId, f.id, "lease F")
		spec.eq(#received.calls, 1, "F applied E's message")
		spec.eq(after.payload.coins, 101, "to the payload")
		spec.list(after.seen, { id }, "and acknowledged it")
		spec.eq(after.schemaVersion > 0, true, "the stored version stays above E's")
		spec.eq(DM.session(f, pf).value.career.jobs[2].tips, 0, "F's value holds the filled field")
	end,
	["rule 77 clearQuarantined and send work on a newer record without touching its payload or lease"] = function()
		local backend, e = D2.world(1, { version = { version = 2 } })
		local id = D2.id(D2.WALL - 20, "q1")
		local lease = { token = "foreign", serverId = "x", jobId = "y", expiresAt = D2.WALL + 1000 }
		D2.seed(backend, nil, {
			schemaVersion = 9,
			epoch = 4,
			lease = lease,
			payload = { coins = 7, gems = 0 },
			quarantine = { D2.item(id, "grant", { amount = 6 }, { reason = "attempts", attempts = 5, generation = 5 }) },
		})
		local cleared = e.run(function()
			return e.store.inbox:clearQuarantined(USER, { id }, "retry")
		end)
		backend:advance(1)
		spec.eq(cleared.value.kind, "ok", "clearQuarantined works")
		spec.eq(cleared.value.value, 1, "one item moved")
		local envelope = DM.stored(backend)
		spec.eq(#envelope.quarantine, 0, "it left the quarantine")
		spec.eq(#envelope.inbox, 1, "and is live again")
		spec.eq(envelope.inbox[1].generation, 6, "at the next generation")
		spec.eq(envelope.schemaVersion, 9, "the version is untouched")
		spec.eq(envelope.epoch, 4, "so is the epoch")
		DM.deepEq(envelope.lease, lease, "and the lease")
		DM.deepEq(envelope.payload, { coins = 7, gems = 0 }, "and the payload")
	end,
}
