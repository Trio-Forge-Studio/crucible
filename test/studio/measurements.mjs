export const measurements = String.raw`
local include = game.ReplicatedStorage.rbxts_include
local TS = require(include.RuntimeLib)
local original = include.node_modules["@trioforge"].crucible.out.internal
local function world(profile)
    local modules = original:Clone()
    modules.Name = "S2Measurement"
    modules.Parent = game.ServerStorage
    local life = TS.import(script, modules, "lifecycle")
    local DenseList = TS.import(script, modules, "dense-list").DenseList
    local members = DenseList.new()
    local scripts, failures = {}, {}
    local joins, leaves, visits, steps, hookSeconds = 0, 0, 0, 0, 0
    local function noop() end
    life.useEffects({
        construct = function() return {} end,
        hook = function(s, hook)
            return function()
                steps += 1
                local began = if profile then os.clock() else 0
                local callback = scripts[s] and scripts[s][hook]
                if callback then callback(s) end
                if profile then hookSeconds += os.clock() - began end
            end
        end,
        start = function(s)
            local callback = scripts[s] and scripts[s].start
            if callback then callback(s) end
        end,
        join = function(s) joins += 1; members:add(s) end,
        leave = function(s) leaves += 1; members:remove(s) end,
        tick = noop, untick = noop, ended = noop,
        reconcile = function() return {} end,
        defer = task.defer, cancel = task.cancel,
        failed = function(message) table.insert(failures, message) end,
        diagnose = noop,
    })
    local function make(callbacks)
        local s = life.track({kind = 1, name = "S2Measurement"})
        scripts[s] = callbacks or {}
        return s
    end
    return {life = life, make = make, members = members, failures = failures,
        counts = function() return joins, leaves, visits, steps end,
        hookSeconds = function() return hookSeconds end,
        walk = function() members:forEach(function() visits += 1 end) end,
        close = function() modules:Destroy() end}
end
for trial = 0, 5 do
    local w = world(trial == 0)
    local states = {}
    for i = 1, 1000 do states[i] = w.make() end
    w.life.admit(states)
    local elapsed, walkSeconds = 0, 0
    for frame = 1, 105 do
        game:GetService("RunService").Heartbeat:Wait()
        local began = os.clock()
        for i = 1, 500 do w.life.request(states[i], 1) end
        for i = 1, 500 do w.life.request(states[i], 0) end
        local walkBegan = if trial == 0 then os.clock() else 0
        w.walk()
        if trial == 0 and frame > 5 then walkSeconds += os.clock() - walkBegan end
        if frame > 5 then elapsed += os.clock() - began end
        assert(w.members:size() == 1000)
    end
    local joins, leaves, visits = w.counts()
    assert(joins == 53500 and leaves == 52500 and visits == 105000 and #w.failures == 0)
    if trial == 0 then
        print(string.format("S2 profile pool totalMs=%.6f walkMs=%.6f lifecycleMs=%.6f hookBodyMs=%.6f", elapsed * 1000, walkSeconds * 1000, (elapsed - walkSeconds) * 1000, w.hookSeconds() * 1000))
    else
        print(string.format("S2 measure pool trial=%d ms/frame=%.6f frames=100 members=1000 disables=500 enables=500 joins=%d leaves=%d visits=%d errors=0", trial, elapsed * 10, joins, leaves, visits))
    end
    w.close()
    local batches = world(trial == 0)
    local created = 0
    local function start()
        created += 1
        assert(created < 2000)
        batches.life.admit({batches.make({start = start})})
    end
    local began = os.clock()
    batches.life.admit({batches.make({start = start})})
    local ms = (os.clock() - began) * 1000
    assert(created == 1024 and #batches.failures == 1 and string.find(batches.failures[1], "1024 batches", 1, true))
    if trial == 0 then
        local _, _, _, steps = batches.counts()
        print(string.format("S2 profile MAX_BATCHES totalMs=%.6f hookBodyMs=%.6f hookCalls=%d", ms, batches.hookSeconds() * 1000, steps))
    else
        print(string.format("S2 measure MAX_BATCHES trial=%d ms=%.6f starts=%d expectedFailures=1", trial, ms, created))
    end
    batches.close()
    local chain = world(trial == 0)
    local a, b
    local cycling = false
    local requests = 0
    local function ask(s, want)
        requests += 1
        assert(requests < 100000)
        chain.life.request(s, want)
    end
    a = chain.make({[2] = function() if cycling then ask(b, 0) end end,
        [4] = function() if cycling then ask(b, 1) end end,
        start = function() cycling = true; ask(a, 1) end})
    b = chain.make({[1] = function(s) b = s end,
        [2] = function() if cycling then ask(a, 1) end end,
        [4] = function() if cycling then ask(a, 0) end end})
    began = os.clock()
    chain.life.admit({a, b})
    ms = (os.clock() - began) * 1000
    assert(requests >= 65536 and #chain.failures == 1 and string.find(chain.failures[1], "65536 requests", 1, true))
    if trial == 0 then
        local _, _, _, steps = chain.counts()
        print(string.format("S2 profile MAX_CHAINED_REQUESTS totalMs=%.6f hookBodyMs=%.6f hookCalls=%d", ms, chain.hookSeconds() * 1000, steps))
    else
        print(string.format("S2 measure MAX_CHAINED_REQUESTS trial=%d ms=%.6f requests=%d expectedFailures=1", trial, ms, requests))
    end
    chain.close()
end
print("S2 measurements complete")
`;
