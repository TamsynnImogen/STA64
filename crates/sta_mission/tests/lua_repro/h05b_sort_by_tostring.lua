-- H05b: tostring used as a sort key / tie-breaker (a game-visible order built from addresses).
MMM.register(function()
  local items = {}
  for i = 1, 12 do items[i] = { id = i } end
  table.sort(items, function(a, b) return tostring(a) < tostring(b) end)
  local ids = {}
  for i = 1, #items do ids[i] = items[i].id end
  obs(join(ids))
end)
