-- H05: tostring of tables, functions, C functions, threads, userdata.
MMM.register(function()
  local t = {}
  local f = function() end
  local co = coroutine.create(f)
  obs(join({ tostring(t), tostring(f), tostring(print), tostring(co), tostring(MMM), tostring(string.format), tostring(_VERSION), type(jit) }, " "))
end)
