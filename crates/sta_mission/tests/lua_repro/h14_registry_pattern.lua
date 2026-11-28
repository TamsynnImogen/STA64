-- H14: the pattern a hand-written mission is likely to use: a registry keyed by name strings,
-- iterated with pairs to run handlers; each handler appends to a shared event list whose order
-- is the game-visible result (as commands queued in that order would be).
local handlers = {}
local events = {}
for _, name in ipairs({ "alpha", "beta", "gamma", "delta", "epsilon", "zeta", "eta", "theta", "iota", "kappa" }) do
  handlers[name] = function(t) events[#events + 1] = name .. ":" .. t end
end
local n = 0
MMM.register(function()
  n = n + 1
  events = {}
  for name, h in pairs(handlers) do h(n) end
  obs(join(events))
end)
