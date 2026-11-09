-- Per-frame Ikemen-GO state capture for the M.A.N.A.T.O.-RS oracle harness.
-- This file is installed into a throwaway runtime copy and is not committed.
local out_path = os.getenv("IKEMEN_ORACLE_OUT")
local scenario = os.getenv("IKEMEN_ORACLE_SCENARIO") or "idle"
local last_frame = -1
local injected = false
local started = false

local function num(v)
    if type(v) == "boolean" then return v and 1 or 0 end
    if type(v) == "number" then return v end
    return 0
end

local function snap()
    return table.concat({
        num(life()), num(lifeMax()), num(power()), num(redLife()),
        num(stateNo()), num(anim()), num(animTime()),
        string.format("%.4f", posX()), string.format("%.4f", posY()),
        string.format("%.4f", velX()), string.format("%.4f", velY()),
        num(facing()), num(ctrl()), num(stateType()), num(moveType()),
        num(hitCount()), num(uniqHitCount()),
        num(guardPoints()), num(dizzyPoints()),
        num(getHitVar("damage")), num(getHitVar("hittime")),
    }, ",")
end

local HEADER = "frame,gameTime,roundState,roundTime,"
    .. "p1_life,p1_lifeMax,p1_power,p1_redLife,p1_state,p1_anim,p1_animTime,"
    .. "p1_x,p1_y,p1_vx,p1_vy,p1_facing,p1_ctrl,p1_stateType,p1_moveType,"
    .. "p1_hitCount,p1_uniqHitCount,p1_guardPoints,p1_dizzyPoints,"
    .. "p1_hitDamage,p1_hitTime,"
    .. "p2_life,p2_lifeMax,p2_power,p2_redLife,p2_state,p2_anim,p2_animTime,"
    .. "p2_x,p2_y,p2_vx,p2_vy,p2_facing,p2_ctrl,p2_stateType,p2_moveType,"
    .. "p2_hitCount,p2_uniqHitCount,p2_guardPoints,p2_dizzyPoints,"
    .. "p2_hitDamage,p2_hitTime"

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
    if scenario == "kfm_attack" and not injected and num(roundState()) == 2 then
        player(1)
        changeState(200)
        injected = true
    end

    player(1)
    local p1 = snap()
    player(2)
    local p2 = snap()

    write_line(table.concat({
        frame, num(gameTime()), num(roundState()), num(roundTime()), p1, p2
    }, ","))
    return ""
end

loadDebugInfo({ "oracle_capture" })
toggleDebugDisplay(true)
