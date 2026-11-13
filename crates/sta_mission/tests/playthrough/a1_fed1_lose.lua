-- Armada I, Federation 1: the lose condition. The player builds nothing and keeps flying the
-- constructor, its only builder, at the nearest Dominion ship; once it is destroyed no
-- constructor, starbase or mining station is left, and the mission is lost.
wait_until(control, 300)
note("control released")
local t0 = now()
while label("Fedcon1") and now() - t0 < 900 do
  local con = label("Fedcon1")
  local dom = nearest(con, units{odf = "zjemdest", enemy = true})
  if dom then move(con, dom.x, dom.z) end
  wait(5)
end
note(label("Fedcon1") and "constructor still alive" or "constructor lost")
wait(600)
