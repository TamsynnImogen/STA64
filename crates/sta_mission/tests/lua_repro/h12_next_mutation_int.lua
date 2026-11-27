-- H12: next/pairs on an integer-keyed table that is mutated during the traversal
-- (assign nil to existing = allowed; adding keys = undefined behaviour in Lua).
local n = 0
MMM.register(function()
  n = n + 1
  local T = {}
  for i = 1, 30 do T[i * 3] = i end
  local out = {}
  for k, v in pairs(T) do
    out[#out + 1] = k
    if v % 2 == 0 then T[k] = nil end
  end
  local T2 = {}
  for i = 1, 20 do T2[i] = i end
  local ok, err = pcall(function()
    for k, v in pairs(T2) do
      out[#out + 1] = -k
      if k < 40 then T2[k + 100 * n] = v end
      if #out > 400 then error("runaway") end
    end
  end)
  local cnt = 0
  for _ in pairs(T2) do cnt = cnt + 1 end
  obs(join(out) .. " ok=" .. tostring(ok) .. " cnt=" .. cnt)
end)
