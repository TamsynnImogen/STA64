-- H10: coroutines driven in a fixed (integer-indexed) order, resumed across ticks, with a
-- yield inside pcall and a wrap generator. Expected reproducible.
local cos = {}
for i = 1, 8 do
  cos[i] = coroutine.create(function(a)
    local x = a
    while true do
      x = x + i
      local ok, got = pcall(coroutine.yield, x)
      x = x + (got or 0)
    end
  end)
end
local gen = coroutine.wrap(function() local k = 0 while true do k = k + 3 coroutine.yield(k) end end)
local n = 0
MMM.register(function()
  n = n + 1
  local out = {}
  for i = 1, 8 do
    local ok, v = coroutine.resume(cos[i], n)
    out[#out + 1] = v
  end
  out[#out + 1] = gen()
  obs(join(out))
end)
