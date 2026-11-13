-- Armada I, Federation 1 "Premonitions": a competent player.
-- Objectives (F1OB1..3): build a starbase and a mining station, defeat the Dominion
-- attack, then destroy the Borg; the Enterprise, the Premonition and the bases survive.
local function built(odf) for _, u in ipairs(units{mine = true, odf = odf}) do if not u.building then return true end end return false end
local function warships() local o = {} for _, u in ipairs(units{mine = true}) do if u.odf ~= "fconst" and u.odf ~= "fbase" and u.odf ~= "fmining" and u.odf ~= "ffreight" and u.odf ~= "fyard" and u.maxhull > 0 and not u.building then o[#o + 1] = u end end return o end
local function hunt(odfs, timeout)
  local t0 = now()
  local last = {}
  while now() - t0 < timeout do
    local foes = {}
    for _, odf in ipairs(odfs) do for _, u in ipairs(units{odf = odf, enemy = true}) do foes[#foes + 1] = u end end
    if #foes == 0 then return true end
    for _, s in ipairs(warships()) do
      local f = nearest(s, foes)
      if f and last[s.id] ~= f.id then attack(s, f) last[s.id] = f.id end
    end
    wait(5)
  end
  note("hunt timed out: " .. table.concat(odfs, ","))
  return false
end

wait_until(control, 300)
note("control released")
local con = label("Fedcon1")
build(con, "fbase", 5450, 1150)
wait_until(function() return built("fbase") end, 400)
note("starbase built")
con = label("Fedcon1")
build(con, "fmining", 5850, 1250)
wait_until(function() return built("fmining") end, 400)
note("mining station built")
-- the base builds a freighter; a shipyard builds escorts while the fleet fights
local base = units{mine = true, odf = "fbase"}[1]
if base then build(base, "ffreight") end
con = label("Fedcon1")
build(con, "fyard", 5300, 1350)
wait_until(function() return built("fyard") end, 400)
note("shipyard built")
local yard = units{mine = true, odf = "fyard"}[1]
for _ = 1, 4 do build(yard, "fdestroy") end
hunt({ "zjemdest" }, 900)
note("Dominion destroyed")
-- The Premonition comes out of a gate in the north-west, chased by the Borg: the fleet
-- sets off at once, before the cineractive locks the controls.
local rescue = paths.PREMOSTART[1]
move(warships(), rescue.x, rescue.z)
wait_until(function() return #units{ like = "bcruise", enemy = true } > 0 end, 300)
wait_until(control, 300)
note("Borg here")
for _, w in ipairs(warships()) do note(string.format("%s #%d at %.0f from the rescue point, hull %.0f", w.odf, w.id, dist(w, rescue), w.hull)) end
hunt({ "bcruise1", "bcruise2" }, 900)
note("Borg destroyed")
wait(600)
