-- H07: math.random (the runtime replaces it with a seeded generator), math.randomseed, and
-- A2.RandomGetUnsigned which shares the stream.
local n = 0
MMM.register(function()
  n = n + 1
  local out = {}
  for i = 1, 5 do out[#out + 1] = math.random(1, 1000000) end
  out[#out + 1] = string.format("%.17g", math.random())
  local ok, v = pcall(function() return A2.RandomGetUnsigned(1000) end)
  out[#out + 1] = tostring(ok) .. ":" .. tostring(v)
  if n == 30 then math.randomseed(99) end
  obs(join(out, " "))
end)
