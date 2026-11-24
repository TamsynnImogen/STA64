#!/usr/bin/env bash
# Play every testbed scenario headless (TOOL-24): each map's mission under its
# own group mod and under Sandbox, driven by the testbed's test plans, and
# report script errors, unsupported calls and how the mission ended.
#   tools/testbed/play_scenarios.sh <testbed repo> [bundle root]
set -uo pipefail
here=$(cd "$(dirname "$0")/../.." && pwd)
src=$(cd "${1:?usage: play_scenarios.sh <testbed repo> [bundle root]}" && pwd)
root=${2:-$src/build/check}
plans=$src/tools/plans
declare -A plan=([tb_combat]=attack_all [tb_defence]=attack_all [tb_behaviour]=attack_all [tb_fleet]=attack_all
                 [tb_economy]=mine [tb_production]=build_battleship [tb_sandbox]=attack_all)
declare -A group=([tb_combat]=Combat [tb_defence]=Defence [tb_behaviour]=Behaviour [tb_fleet]=Fleet
                  [tb_economy]=Economy [tb_production]=Production [tb_sandbox]=Sandbox)
bad=0
for m in tb_combat tb_defence tb_behaviour tb_fleet tb_economy tb_production tb_sandbox; do
    for mod in "STA64Testbed_${group[$m]}" STA64Testbed_Sandbox; do
        out=$("$here/tools/agent-cargo" run -q -p sta_mission --example mission_playthrough -- \
            --install "$root" --mod "$mod" --mission "$m" --plan "$plans/${plan[$m]}.lua" --ticks 18000 2>&1)
        errs=$(grep -c '"kind":"\(error\|unsupported\|plan-error\)"' <<<"$out")
        end=$(grep -o '"kind":"end","what":"[^"]*' <<<"$out" | sed 's/.*"what":"//')
        printf '%-14s %-26s errors=%-3s end=%s\n' "$m" "$mod" "$errs" "${end:-none in 10 min}"
        if [[ $errs != 0 ]]; then grep '"kind":"\(error\|unsupported\|plan-error\)"' <<<"$out" | head -3; bad=1; fi
        [[ $m == tb_sandbox ]] && break
    done
done
exit $bad
