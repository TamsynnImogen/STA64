-- H03: pairs over a table whose keys are closures (hash = address).
local T = {}
for i = 1, 40 do T[function() return i end] = i end
MMM.register(function()
  local out = {}
  for f, v in pairs(T) do out[#out + 1] = f() end
  obs(join(out))
end)
