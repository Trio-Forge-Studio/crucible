local spec = require("./lib/load")
local Engine = require("./lib/engine")
local World = require("./lib/world")
local new = require("./lib/scheduler-world")
local meter = require("./lib/meter")
local F = Engine.FrameEvent
local STEP = 1 / 60
local tests = {}

function tests.lanes_use_PreSimulation_and_PostSimulation_with_the_whole_update_lane_before_lateUpdate()
	local w = new()
	local seen = {}
	local frameDt = 0.031
	for _, name in { "A", "B" } do
		local methods = {}
		for _, hook in { "fixedUpdate", "update", "lateUpdate", "preRender" } do
			methods[hook] = function(_, dt)
				spec.eq(dt, if hook == "fixedUpdate" then STEP else frameDt, hook .. " dt")
				table.insert(seen, hook .. name)
			end
		end
		w.member(name, methods)
	end
	w.engine.frame(F.PreAnimation, STEP)
	w.engine.frame(F.Heartbeat, STEP)
	spec.list(seen, {}, "unrelated events")
	w.engine.frame(F.PreSimulation, STEP)
	w.engine.frame(F.PostSimulation, frameDt)
	w.engine.frame(F.PreRender, frameDt)
	spec.list(seen, {
		"fixedUpdateA",
		"fixedUpdateB",
		"updateA",
		"updateB",
		"lateUpdateA",
		"lateUpdateB",
		"preRenderA",
		"preRenderB",
	}, "lane order")
end

function tests.S1_bindings_ascend_by_priority_equal_priorities_keep_bind_order_before_RenderStepped_then_newest_PreRender()
	local w = new()
	local seen = {}
	local function log(name)
		return function()
			table.insert(seen, name)
		end
	end
	w.engine.port.onFrame(F.PreRender, log("old"))
	w.member("plain", { preRender = log("plain") })
	w.engine.port.onFrame(F.PreRender, log("new"))
	for _, priority in { 2000, 201, 200, 199, 100, 0 } do
		w.member(tostring(priority), { preRender = log(tostring(priority)) }, priority)
		if priority == 200 then
			w.engine.port.bindRenderStep("external", 200, log("equal"))
			w.member("same lane", { preRender = log("same lane") }, 200)
		end
	end
	w.engine.port.onFrame(F.RenderStepped, log("RenderStepped"))
	w.engine.frame(F.PreRender, STEP)
	spec.list(
		seen,
		{ "0", "100", "199", "200", "same lane", "equal", "201", "2000", "RenderStepped", "new", "plain", "old" },
		"S1 row 3"
	)
	spec.eq(#w.engine.renderSteps, 7, "one Crucible binding per priority plus external")
end

function tests.S1_client_render_then_PreAnimation_PreSimulation_PostSimulation_Heartbeat_and_server_omits_render()
	for _, renders in { true, false } do
		local w = new(renders)
		local seen = {}
		for name, event in F do
			if name ~= "RenderStepped" then
				w.engine.port.onFrame(event, function()
					table.insert(seen, name)
				end)
			end
		end
		w.engine.cycle(STEP, renders)
		local expected = { "PreAnimation", "PreSimulation", "PostSimulation", "Heartbeat" }
		if renders then
			table.insert(expected, 1, "PreRender")
		end
		spec.list(seen, expected, "S1 frame order")
	end
end

function tests.server_refuses_preRender_or_renderPriority_and_reports_through_the_port()
	local w = new(false)
	local seen = 0
	local function ran()
		seen += 1
	end
	w.member("hook", { preRender = ran, update = ran })
	w.member("priority", { update = ran }, 0)
	w.member("valid", { update = ran })
	w.engine.cycle(STEP, true)
	spec.eq(seen, 1, "only valid server class bound")
	spec.eq(#w.engine.renderSteps, 0, "no bindings")
	spec.eq(#w.engine.frames[F.PreRender].list, 0, "no plain connection")
	spec.eq(#w.engine.logs, 2, "both forms reported")
	for _, log in w.engine.logs do
		spec.eq(log.level, 1, "Error")
	end
end

function tests.throw_or_yield_faults_only_its_owner_and_the_rest_of_each_lane_runs_in_order()
	for _, hook in { "preRender", "fixedUpdate", "update", "lateUpdate" } do
		for _, fail in
			{
				function()
					error("frame broke")
				end,
				coroutine.yield,
			}
		do
			local w = new()
			local seen = {}
			local states = {}
			for _, name in { "A", "B", "C" } do
				states[name] = w.member(name, {
					[hook] = function()
						table.insert(seen, name)
						if name == "B" then
							fail()
						end
					end,
				})
			end
			w.engine.cycle(STEP, true)
			spec.list(seen, { "A", "B", "C" }, hook)
			spec.eq(states.B.phase, World.Phase.Faulted, "only B faulted")
			spec.eq(states.B.fault.hook, World.Hook.Frame, "frame blame")
			spec.eq(states.A.phase, World.Phase.Enabled, "A enabled")
			spec.eq(states.C.phase, World.Phase.Enabled, "C enabled")
		end
	end
end

function tests.scripted_dt_keeps_fraction_drops_excess_steps_and_resets_carry_when_empty()
	local w = new(true, 64)
	local step = 1 / 64
	local count = 0
	local s = w.member("fixed", {
		fixedUpdate = function(_, dt)
			spec.eq(dt, step, "constant step")
			count += 1
		end,
	})
	for _, frame in
		{
			{ step * 0.5, 0 },
			{ step * 0.75, 1 },
			{ step * 0.25, 1 },
			{ step * 0.5, 2 },
			{ step * 10.5, 6 },
			{ step * 0.5, 7 },
		}
	do
		w.engine.frame(F.PreSimulation, frame[1])
		spec.eq(count, frame[2], "whole steps")
	end
	w.engine.frame(F.PreSimulation, step * 0.25)
	w.life.request(s, World.Want.Disabled)
	w.life.request(s, World.Want.Enabled)
	w.engine.frame(F.PreSimulation, step * 0.75)
	spec.eq(count, 7, "empty lane discards carry")
	w.engine.frame(F.PreSimulation, step)
	spec.eq(count, 8, "fixed lane reconnects")
end

function tests.fixed_carry_survives_unticking_another_fixed_or_non_fixed_member()
	for _, hook in { "fixedUpdate", "update" } do
		local w = new(true, 64)
		local step = 1 / 64
		local count = 0
		w.member("stays enabled", {
			fixedUpdate = function()
				count += 1
			end,
		})
		local other = w.member("unticked", { [hook] = function() end })
		w.engine.frame(F.PreSimulation, step * 0.25)
		spec.eq(count, 0, "partial step")
		w.life.request(other, World.Want.Disabled)
		w.engine.frame(F.PreSimulation, step * 0.75)
		spec.eq(count, 1, hook .. " untick preserves another member's carry")
	end
end

function tests.disable_removes_both_update_and_lateUpdate_and_disconnects_PostSimulation()
	local w = new()
	local seen = {}
	local s = w.member("both lanes", {
		update = function()
			table.insert(seen, "update")
		end,
		lateUpdate = function()
			table.insert(seen, "lateUpdate")
		end,
	})
	w.engine.frame(F.PostSimulation, STEP)
	spec.list(seen, { "update", "lateUpdate" }, "enabled hooks run")
	table.clear(seen)
	w.life.request(s, World.Want.Disabled)
	w.engine.frame(F.PostSimulation, STEP)
	spec.list(seen, {}, "disabled hooks do not run")
	spec.eq(#w.engine.frames[F.PostSimulation].list, 0, "both lanes disconnected")
end

function tests.emptying_one_render_priority_keeps_the_other_priority_running()
	local w = new()
	local seen = {}
	local a = w.member("A", {
		preRender = function()
			table.insert(seen, "A")
		end,
	}, 100)
	w.member("B", {
		preRender = function()
			table.insert(seen, "B")
		end,
	}, 200)
	w.engine.frame(F.PreRender, STEP)
	spec.list(seen, { "A", "B" }, "both priorities run")
	w.life.request(a, World.Want.Disabled)
	table.clear(seen)
	w.engine.frame(F.PreRender, STEP)
	spec.list(seen, { "B" }, "other priority stays bound")
	spec.eq(#w.engine.renderSteps, 1, "only the empty lane unbound")
end

function tests.a_frame_hook_that_calls_remove_finishes_before_teardown_and_no_removed_member_ticks_again()
	local w = new()
	local seen = {}
	local a, b
	local function extra(name)
		return {
			onDisable = function()
				table.insert(seen, "disable" .. name)
			end,
			onDestroy = function()
				table.insert(seen, "destroy" .. name)
			end,
		}
	end
	a = w.member("A", {
		update = function()
			table.insert(seen, "A")
			w.life.request(a, World.Want.Destroyed)
			w.life.request(b, World.Want.Destroyed)
			table.insert(seen, "continued")
		end,
		lateUpdate = function()
			error("ended A")
		end,
	}, nil, extra("A"))
	b = w.member("B", {
		update = function()
			error("ended B")
		end,
		lateUpdate = function()
			error("ended B late")
		end,
	}, nil, extra("B"))
	w.engine.frame(F.PostSimulation, STEP)
	spec.list(seen, { "A", "continued", "disableA", "destroyA", "disableB", "destroyB" }, "v2 frame runtime case")
	spec.eq(a.phase, World.Phase.Destroyed, "A destroyed")
	spec.eq(b.phase, World.Phase.Destroyed, "B destroyed")
	w.engine.frame(F.PostSimulation, STEP)
	spec.eq(#seen, 6, "never tick again")
end

function tests.disable_then_reenable_during_a_walk_appends_for_next_walk_and_skips_the_old_slot()
	local w = new()
	local seen = {}
	local b
	local once = false
	w.member("A", {
		update = function()
			table.insert(seen, "A")
			if not once then
				w.life.request(b, World.Want.Disabled)
			end
		end,
	})
	b = w.member("B", {
		update = function()
			table.insert(seen, "B")
		end,
	})
	w.member("C", {
		update = function()
			table.insert(seen, "C")
			if not once then
				once = true
				w.life.request(b, World.Want.Enabled)
			end
		end,
	})
	w.engine.frame(F.PostSimulation, STEP)
	spec.list(seen, { "A", "C" }, "ended membership skips the old slot")
	table.clear(seen)
	w.engine.frame(F.PostSimulation, STEP)
	spec.list(seen, { "A", "C", "B" }, "new membership appends in tick order")
end

function tests.disable_during_each_lane_skips_the_next_member_and_later_lanes()
	for _, hook in { "fixedUpdate", "update", "lateUpdate", "preRender" } do
		local w = new()
		local b
		w.member("A", {
			[hook] = function()
				w.life.request(b, World.Want.Disabled)
			end,
		})
		b = w.member("B", {
			[hook] = function()
				error("disabled hook ran")
			end,
		})
		w.engine.cycle(STEP * 2, true)
		spec.eq(b.phase, World.Phase.Disabled, "disabled before next member")
		spec.eq(b.fault, nil, "no disabled hook")
	end
end

function tests.empty_lanes_disconnect_and_rejoin_once_in_new_bind_order()
	local w = new()
	local methods = {
		preRender = function() end,
		fixedUpdate = function() end,
		update = function() end,
		lateUpdate = function() end,
	}
	local a = w.member("A", methods, 200)
	w.scheduler:add(a)
	spec.eq(#w.engine.renderSteps, 1, "idempotent tick")
	w.life.request(a, World.Want.Disabled)
	w.scheduler:remove(a)
	spec.eq(#w.engine.renderSteps, 0, "render disconnected")
	spec.eq(#w.engine.frames[F.PreSimulation].list, 0, "fixed disconnected")
	spec.eq(#w.engine.frames[F.PostSimulation].list, 0, "post disconnected")
	w.life.request(a, World.Want.Enabled)
	spec.eq(#w.engine.renderSteps, 1, "render rebound once")
	spec.eq(#w.engine.frames[F.PostSimulation].list, 1, "one shared post connection")
end

function tests.pool_rule_all_five_lane_kinds_bound_slot_work_and_growth_and_preserve_tick_order()
	for _, laneKind in { "fixedUpdate", "update", "lateUpdate", "preRender", "priority" } do
		local w = new()
		local hook = if laneKind == "priority" then "preRender" else laneKind
		local enabled, pool, seen, added = {}, {}, {}, {}
		local churn = false
		for i = 1, 1500 do
			local s
			s = w.member(tostring(i), {
				[hook] = function()
					table.insert(seen, s.record.name)
					if churn and #added < 500 then
						w.life.request(s, World.Want.Disabled)
						table.insert(pool, s)
						local reuse = table.remove(pool, 1)
						w.life.request(reuse, World.Want.Enabled)
						table.insert(added, reuse)
					end
				end,
			}, if laneKind == "priority" then 100 else nil)
			if i <= 1000 then
				table.insert(enabled, s)
			else
				w.life.request(s, World.Want.Disabled)
				table.insert(pool, s)
			end
		end
		local index = ({ preRender = 0, fixedUpdate = 1, update = 2, lateUpdate = 3 })[hook]
		local lane = if laneKind == "priority" then w.scheduler.renderLanes[100] else w.scheduler.lanes[index]
		local m = meter(lane.members)
		local top
		churn = true
		for frame = 1, 100 do
			table.clear(seen)
			table.clear(added)
			m.reads = 0
			local expected = {}
			for _, s in enabled do
				table.insert(expected, s.record.name)
			end
			w.engine.cycle(STEP, true)
			spec.list(seen, expected, laneKind .. " tick order")
			spec.eq(m.reads <= 6000, true, laneKind .. " bounded slot work")
			if frame == 5 then
				top = m.top
			end
			if frame > 5 then
				spec.eq(m.top, top, "slot growth stops")
			end
			enabled = { table.unpack(enabled, 501) }
			for _, s in added do
				table.insert(enabled, s)
			end
		end
		spec.eq(lane.members:size(), 1000, "constant membership")
	end
end

function tests.only_overridden_lanes_connect_and_suspended_start_does_not_tick_until_it_finishes()
	local w = new()
	w.member("none", {})
	for _, event in { F.PreRender, F.PreSimulation, F.PostSimulation } do
		spec.eq(#w.engine.frames[event].list, 0, "no unused connection")
	end
	local count, suspended = 0, nil
	w.member(
		"waiting",
		{
			update = function()
				count += 1
			end,
		},
		nil,
		{
			start = function()
				suspended = coroutine.running()
				coroutine.yield()
			end,
		}
	)
	w.engine.frame(F.PostSimulation, STEP)
	spec.eq(count, 0, "start still running")
	spec.eq(coroutine.resume(suspended), true, "start finishes")
	w.engine.frame(F.PostSimulation, STEP)
	spec.eq(count, 1, "ticks after start")
end

function tests.S1_rebound_priority_lane_takes_its_new_bind_order_among_equal_priorities()
	local w = new()
	local seen = {}
	local a = w.member("A", {
		preRender = function()
			table.insert(seen, "A")
		end,
	}, 100)
	w.engine.port.bindRenderStep("external", 100, function()
		table.insert(seen, "external")
	end)
	w.engine.frame(F.PreRender, STEP)
	spec.list(seen, { "A", "external" }, "initial bind order")
	w.life.request(a, World.Want.Disabled)
	w.life.request(a, World.Want.Enabled)
	table.clear(seen)
	w.engine.frame(F.PreRender, STEP)
	spec.list(seen, { "external", "A" }, "rebind order")
end

return tests
