local Engine = require("./lib/engine")
local spec = require("./lib/load")

return {
	["act-on 11: render steps run by ascending priority, in bind order for equal priorities"] = function()
		local e = Engine.new()
		local ran = {}
		for _, step in { { "late", 300 }, { "early", 100 }, { "mid", 200 }, { "mid2", 200 } } do
			e.port.bindRenderStep(step[1], step[2], function()
				table.insert(ran, step[1])
			end)
		end
		e.frame(Engine.FrameEvent.PreRender, 1 / 60)
		spec.list(ran, { "early", "mid", "mid2", "late" }, "render order")
	end,

	["act-on 11: leaving the world queues tag removal, tagged skips the instance, and returning queues tag addition"] = function()
		local e = Engine.new()
		local door = e.instance("Door1")
		e.addTag(door, "Door")
		e.addTag(door, "Lamp")
		e.flush()
		local signals = {}
		for _, tag in { "Door", "Lamp" } do
			e.port.onTagRemoved(tag, function(instance)
				table.insert(signals, `removed {tag} {instance.Name}`)
			end)
			e.port.onTagAdded(tag, function(instance)
				table.insert(signals, `added {tag} {instance.Name}`)
			end)
		end
		e.setInWorld(door, false)
		spec.list(signals, {}, "signals are deferred")
		e.flush()
		spec.list(signals, { "removed Door Door1", "removed Lamp Door1" }, "leaving the world")
		spec.eq(#e.port.tagged("Door"), 0, "tagged skips an instance out of the world")
		spec.eq(e.port.hasTag(door, "Door"), true, "the tag stays on the instance")
		table.clear(signals)
		e.setInWorld(door, true)
		e.flush()
		spec.list(signals, { "added Door Door1", "added Lamp Door1" }, "returning to the world")
		spec.eq(e.port.tagged("Door")[1], door, "tagged sees it again")
	end,
}
