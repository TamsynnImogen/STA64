-- H01: pairs over a table with string keys (hash order depends on the per-state string seed).
local T = {}
for i = 1, 64 do T["key" .. i] = i end
local n = 0
MMM.register(function()
  n = n + 1
  local out = {}
  for k in pairs(T) do out[#out + 1] = k end
  obs(join(out))
  T["extra" .. n] = n
end)
