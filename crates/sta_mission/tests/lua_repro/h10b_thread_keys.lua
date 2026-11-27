-- H10b: a scheduler that keeps its coroutines in a table keyed by the thread objects and
-- walks it with pairs (hash = address).
local byco = {}
for i = 1, 8 do
  byco[coroutine.create(function() end)] = i
end
MMM.register(function()
  local order = {}
  for _, i in pairs(byco) do order[#order + 1] = i end
  obs(join(order))
end)
