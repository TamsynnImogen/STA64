-- H02: pairs over a table whose keys are fresh tables (hash = address).
local T, K = {}, {}
for i = 1, 40 do
  local k = { id = i }
  K[i] = k
  T[k] = i
end
MMM.register(function()
  local out = {}
  for _, v in pairs(T) do out[#out + 1] = v end
  obs(join(out))
  local k = { id = 100 + #K }
  K[#K + 1] = k
  T[k] = k.id
end)
