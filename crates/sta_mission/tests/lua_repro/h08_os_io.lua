-- H08: os.time / os.clock / os.date / io, and what a script gets in their place.
MMM.register(function()
  local ok1, a = pcall(function() return os.time() end)
  local ok2, b = pcall(function() return os.clock() end)
  local ok3, c = pcall(function() return os.date("%c") end)
  local ok4, d = pcall(function() return io.open("x") end)
  local ok5, e = pcall(function() return math.randomseed(os.time()) end)
  local ok6, f = pcall(function() return os.time() + 1 end)
  local ok7, g = pcall(function() return collectgarbage("count") end)
  obs(join({ tostring(ok1), type(a), tostring(a), type(b), tostring(b), type(c), tostring(c), type(d), tostring(d),
    tostring(ok5), tostring(ok6), tostring(f), type(os), tostring(os), tostring(ok7), type(g), tostring(g) }, " "))
end)
