-- Per-frame Ikemen-GO state capture for the M.A.N.A.T.O.-RS oracle harness.
-- This file is installed into a throwaway runtime copy and is not committed.
local out_path = os.getenv("IKEMEN_ORACLE_OUT")
local scenario = os.getenv("IKEMEN_ORACLE_SCENARIO") or "idle"
local last_frame = -1
local injected = false
local started = false
local last_inject = -1000
local second_injected = false
local third_injected = false

local function num(v)
    if type(v) == "boolean" then return v and 1 or 0 end
    if type(v) == "number" then return v end
    return 0
end

-- Debug info callbacks must never raise: a single Lua error makes the engine
-- stop calling the provider for the rest of the match.
local function safe(fn, ...)
    local ok, value = pcall(fn, ...)
    if ok then return value end
    return -999
end

local function snap()
    return table.concat({
        num(safe(life)), num(safe(lifeMax)), num(safe(power)), num(safe(redLife)),
        num(safe(stateNo)), num(safe(anim)), num(safe(animTime)),
        string.format("%.4f", num(safe(posX))), string.format("%.4f", num(safe(posY))),
        string.format("%.4f", num(safe(velX))), string.format("%.4f", num(safe(velY))),
        num(safe(facing)), num(safe(ctrl)), num(safe(stateType)), num(safe(moveType)),
        num(safe(hitCount)), num(safe(uniqHitCount)),
        num(safe(guardPoints)), num(safe(dizzyPoints)),
        num(safe(getHitVar, "damage")), num(safe(getHitVar, "hittime")),
        num(safe(moveContact)), num(safe(moveHit)), num(safe(moveGuarded)),
        num(safe(numProj)), num(safe(projContactTime, -1)),
        num(safe(projHitTime, -1)), num(safe(projGuardedTime, -1)),
    }, ",")
end

local HEADER = "frame,gameTime,roundState,roundTime,"
    .. "p1_life,p1_lifeMax,p1_power,p1_redLife,p1_state,p1_anim,p1_animTime,"
    .. "p1_x,p1_y,p1_vx,p1_vy,p1_facing,p1_ctrl,p1_stateType,p1_moveType,"
    .. "p1_hitCount,p1_uniqHitCount,p1_guardPoints,p1_dizzyPoints,"
    .. "p1_hitDamage,p1_hitTime,p1_moveContact,p1_moveHit,p1_moveGuarded,"
    .. "p1_numProj,p1_projContact,p1_projHit,p1_projGuarded,"
    .. "p2_life,p2_lifeMax,p2_power,p2_redLife,p2_state,p2_anim,p2_animTime,"
    .. "p2_x,p2_y,p2_vx,p2_vy,p2_facing,p2_ctrl,p2_stateType,p2_moveType,"
    .. "p2_hitCount,p2_uniqHitCount,p2_guardPoints,p2_dizzyPoints,"
    .. "p2_hitDamage,p2_hitTime,p2_moveContact,p2_moveHit,p2_moveGuarded,"
    .. "p2_numProj,p2_projContact,p2_projHit,p2_projGuarded,p2_juggle"

local function write_line(line)
    local f = io.open(out_path, "a")
    if f then
        f:write(line, "\n")
        f:close()
    end
end

function oracle_capture()
    local frame = getFrameCount()
    if frame == last_frame then
        return ""
    end
    last_frame = frame
    if not started then
        started = true
        local f = io.open(out_path, "w")
        if f then
            f:write(HEADER, "\n")
            f:close()
        end
    end

    -- Scenario: force P1 into KFM state 200 once the Fight phase starts.
    if scenario == "kfm_multihit" and not injected and num(roundState()) == 2 then
        player(1)
        changeState(7000)
        injected = true
    elseif scenario == "kaen_projectile" and not injected and num(roundState()) == 2 then
        player(1)
        changeState(1000)
        injected = true
    elseif scenario == "kaen_projectile_guard" and not injected and num(roundState()) == 2 then
        player(1)
        changeState(1000)
        last_inject = frame
        injected = true
    elseif scenario == "kaen_projectile_guard" and injected and not guard_set
        and frame - last_inject >= 2 then
        player(2)
        changeState(150)
        guard_set = true
    elseif scenario == "kfm_attack" and not injected and num(roundState()) == 2 then
        player(1)
        changeState(200)
        injected = true
    elseif scenario == "kfm_launch" and not injected and num(roundState()) == 2 then
        player(1)
        changeState(440)
        injected = true
    elseif scenario == "kfm_juggle" and not injected and num(roundState()) == 2 then
        player(1)
        changeState(440)
        injected = true
    elseif scenario == "kfm_juggle" and injected and not second_injected then
        player(2)
        if num(stateNo()) >= 5071 then
            player(1)
            changeState(210)
            second_injected = true
        end
    elseif scenario == "kfm_juggle_exhaust" and not injected and num(roundState()) == 2 then
        player(1)
        changeState(440)
        injected = true
    elseif scenario == "kfm_juggle_exhaust" and injected and not second_injected then
        player(2)
        if num(stateNo()) >= 5071 then
            player(1)
            changeState(210)
            second_injected = true
        end
    elseif scenario == "kfm_juggle_exhaust" and second_injected and not third_injected then
        player(2)
        if num(life()) < 928 and num(stateNo()) >= 5020 then
            player(1)
            changeState(240)
            third_injected = true
        end
    elseif scenario == "kfm_attack_loop" and num(roundState()) == 2
        and frame - last_inject >= 45 then
        player(1)
        if num(stateNo()) == 0 then
            changeState(200)
            last_inject = frame
        end
    elseif scenario == "kfm_guard" and num(roundState()) == 2
        and frame - last_inject >= 45 then
        player(1)
        if num(stateNo()) == 0 then
            changeState(200)
            player(2)
            changeState(150)
            last_inject = frame
        end
    end

    player(1)
    local p1 = snap()
    player(2)
    local p2 = snap()
    local p2_id = safe(id)
    player(1)
    local p2_juggle = safe(jugglePoints, p2_id)

    write_line(table.concat({
        frame, num(gameTime()), num(roundState()), num(roundTime()), p1, p2, p2_juggle
    }, ","))
    return ""
end

loadDebugInfo({ "oracle_capture" })
toggleDebugDisplay(true)
