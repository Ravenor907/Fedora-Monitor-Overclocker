-- Exercises a monitor-oc generated Gamescope script the way Gamescope does:
-- run the system display scripts, then the user script, then LookupDisplay()
-- and drm_set_refresh() (which calls dynamic_modegen with the preferred mode).
--
-- usage: luajit gamescope_harness.lua <script.lua> <state file> <report file>

local script_path, state_path, report_path = arg[1], arg[2], arg[3]
assert(script_path and state_path and report_path, "usage: harness <script> <state> <report>")

local failures = 0
local function check(cond, what)
    if not cond then
        failures = failures + 1
        io.stderr:write("FAIL: " .. what .. "\n")
    end
end

local function write(path, text)
    local f = assert(io.open(path, "w"))
    f:write(text)
    f:close()
end

local function read_kv(path)
    local out = {}
    local f = io.open(path, "r")
    if not f then return out end
    for line in f:lines() do
        local k, v = line:match("^([%w_]+)=(.*)$")
        if k then out[k] = v end
    end
    f:close()
    return out
end

-- Panel timings modelled on a 1920x1080 120 Hz handheld LCD: fixed pixel
-- clock, refresh set by the vertical front porch (like Gamescope's ROG Ally profile).
local HTOTAL, VSYNC, VBP = 2080, 3, 8
local NATIVE_VFP = 54
local VTOTAL = 1080 + NATIVE_VFP + VSYNC + VBP
local CLOCK = math.floor(HTOTAL * VTOTAL * 120 / 1000)

local function preferred_mode()
    return {
        clock = CLOCK,
        hdisplay = 1920, hsync_start = 1968, hsync_end = 2000, htotal = HTOTAL,
        vdisplay = 1080, vsync_start = 1080 + NATIVE_VFP, vsync_end = 1080 + NATIVE_VFP + VSYNC, vtotal = VTOTAL,
        vrefresh = 120,
    }
end

local function refresh_of(mode)
    return mode.clock * 1000 / (mode.htotal * mode.vtotal)
end

-- A fresh Gamescope Lua environment with Gamescope's helpers and one stock profile.
local function fresh_gamescope(stock)
    _G.gamescope = {
        config = { known_displays = {} },
        log_priority = { error = 1, warning = 2, info = 3, debug = 4 },
        log = function() end,
        eotf = { gamma22 = 0 },
        modegen = {},
    }
    function _G.zero_index(i) return i + 1 end
    function _G.debug() end
    function _G.warn() end
    function _G.info() end
    gamescope.modegen.adjust_front_porch = function(mode, vfp)
        local vsync = mode.vsync_end - mode.vsync_start
        local vbp = mode.vtotal - mode.vsync_end
        mode.vsync_start = mode.vdisplay + vfp
        mode.vsync_end = mode.vsync_start + vsync
        mode.vtotal = mode.vsync_end + vbp
    end
    gamescope.modegen.calc_vrefresh = function(mode)
        return math.floor((1000 * mode.clock) / (mode.htotal * mode.vtotal))
    end

    if stock then
        local rates = {}
        for r = 48, 120 do table.insert(rates, r) end
        gamescope.config.known_displays.stock_lcd = {
            pretty_name = "Stock LCD",
            hdr = { supported = false },
            dynamic_refresh_rates = rates,
            dynamic_modegen = function(base_mode, refresh)
                local mode = base_mode
                local vfp = math.floor(mode.clock * 1000 / (mode.htotal * refresh)) - 1080 - VSYNC - VBP
                gamescope.modegen.adjust_front_porch(mode, vfp)
                mode.vrefresh = gamescope.modegen.calc_vrefresh(mode)
                mode.stock_generated = true
                return mode
            end,
            matches = function(display)
                if display.vendor == "TMX" and display.product == 2 and display.model == "TL070FVXS01-0" then
                    return 5000
                end
                if display.vendor == "BOE" and display.product == 0x0C33 and display.data_string == "TS070FHM-LU0" then
                    return 5000
                end
                return -1
            end,
        }
    end
end

-- Gamescope's CScriptManager::LookupDisplay.
local function lookup(display)
    local best, key, prio = nil, nil, -1
    for k, d in pairs(gamescope.config.known_displays) do
        if type(d.matches) == "function" then
            local p = d.matches(display)
            if p > prio then best, key, prio = d, k, p end
        end
    end
    return best, key
end

local TMX = { vendor = "TMX", product = 2, model = "TL070FVXS01-0", data_string = "" }
local BOE = { vendor = "BOE", product = 0x0C33, model = "", data_string = "TS070FHM-LU0" }

local function load(stock)
    fresh_gamescope(stock)
    dofile(script_path)
end

local function rates_max(d)
    local m = 0
    for _, r in ipairs(d.dynamic_refresh_rates) do m = math.max(m, r) end
    return m
end

-- 1. Nothing configured: the stock profile stays in charge.
os.remove(state_path)
os.remove(report_path)
load(true)
local d, key = lookup(TMX)
check(key == "stock_lcd", "no state: stock profile should win, got " .. tostring(key))
check(read_kv(report_path).extended == "no", "no state: report should say extended=no")

-- 2. An armed trial is enabled for exactly one Gamescope start.
write(state_path, "max=0\nmethod=clock\ntrial=135\ntrial_state=armed\n")
load(true)
d, key = lookup(TMX)
check(key == "monitor_oc_panel", "armed trial: our profile should win, got " .. tostring(key))
check(read_kv(state_path).trial_state == "running", "armed trial should be marked running")
check(rates_max(d) == 135, "armed trial: max rate should be 135, got " .. rates_max(d))
check(d.dynamic_refresh_rates[1] == 48, "stock rates should be kept")
check(d.hdr and d.hdr.supported == false, "stock fields (hdr) should be copied")

local m = d.dynamic_modegen(preferred_mode(), 100)
check(m.stock_generated, "<= stock max should use the stock modegen")
check(math.abs(refresh_of(m) - 100) < 0.5, "stock 100 Hz mode refresh " .. refresh_of(m))

-- Not being tested: trial rates are held back at the stock maximum.
m = d.dynamic_modegen(preferred_mode(), 135)
check(math.abs(refresh_of(m) - 120) < 0.5, "untested trial rate should be held at 120, got " .. refresh_of(m))
check(read_kv(report_path).held_back == "135", "report should record the held-back rate")

-- Being tested, but for a different rate or past its deadline: still held back.
write(state_path, "max=0\nmethod=clock\ntrial=135\ntrial_state=running\nlive=130\nlive_until=" .. (os.time() + 60) .. "\n")
m = d.dynamic_modegen(preferred_mode(), 135)
check(math.abs(refresh_of(m) - 120) < 0.5, "a different live rate must not unlock 135")
write(state_path, "max=0\nmethod=clock\ntrial=135\ntrial_state=running\nlive=135\nlive_until=" .. (os.time() - 1) .. "\n")
m = d.dynamic_modegen(preferred_mode(), 135)
check(math.abs(refresh_of(m) - 120) < 0.5, "an expired test must not unlock 135")

-- Being tested: the real rate.
write(state_path, "max=0\nmethod=clock\ntrial=135\ntrial_state=running\nlive=135\nlive_until=" .. (os.time() + 60) .. "\n")
m = d.dynamic_modegen(preferred_mode(), 135)
check(math.abs(refresh_of(m) - 135) < 0.2, "clock method 135 Hz refresh " .. refresh_of(m))
check(m.vtotal == VTOTAL, "clock method should keep the native blanking")
check(m.clock > CLOCK, "clock method should raise the pixel clock")
check(m.vrefresh == 135 or m.vrefresh == 134, "vrefresh field " .. m.vrefresh)
check(read_kv(report_path).last_mode == "1920x1080@135", "report should record the last mode")

m = d.dynamic_modegen(preferred_mode(), 150)
check(m.clock == CLOCK and m.vtotal == VTOTAL, "above the limit should return the native mode")

-- Confirmed in this session: the rate stays available without a test.
write(state_path, "max=135\nmethod=clock\n")
m = d.dynamic_modegen(preferred_mode(), 135)
check(math.abs(refresh_of(m) - 135) < 0.2, "confirmed 135 Hz should be used without a test")
write(state_path, "max=0\nmethod=clock\ntrial=135\ntrial_state=running\n")

-- 3. Gamescope starting again without a confirmation drops the trial.
load(true)
d, key = lookup(TMX)
check(key == "stock_lcd", "unconfirmed trial must expire on the next start, got " .. tostring(key))
check(read_kv(state_path).trial_state == "expired", "trial should be marked expired")

-- 4. A confirmed limit persists across starts.
write(state_path, "max=130\nmethod=clock\n")
load(true)
load(true)
d, key = lookup(TMX)
check(key == "monitor_oc_panel" and rates_max(d) == 130, "confirmed 130 Hz should persist")

-- 5. Porch method: shrink the blanking first, then raise the clock.
write(state_path, "max=140\nmethod=porch\n")
load(true)
d = lookup(TMX)
m = d.dynamic_modegen(preferred_mode(), 123)
check(m.clock == CLOCK, "porch method should keep the clock when blanking allows")
check(math.abs(refresh_of(m) - 123) < 0.2, "porch 123 Hz refresh " .. refresh_of(m))
m = d.dynamic_modegen(preferred_mode(), 140)
check(m.vsync_start - m.vdisplay == 3, "porch method should bottom out at the minimum front porch")
check(m.clock > CLOCK and math.abs(refresh_of(m) - 140) < 0.2, "porch 140 Hz refresh " .. refresh_of(m))

-- 6. The panel variant identified by its EDID data string (not product name).
--    (The script was generated for TMX, so BOE must not match it.)
d, key = lookup(BOE)
check(key == "stock_lcd", "a different panel must not match our profile")

-- 7. No stock profile at all: extend the EDID rates.
write(state_path, "max=130\nmethod=clock\n")
load(false)
d, key = lookup(TMX)
check(key == "monitor_oc_panel", "without a stock profile ours should still register")
check(d.dynamic_refresh_rates[1] == 120 and rates_max(d) == 130, "EDID rates should be extended")
m = d.dynamic_modegen(preferred_mode(), 125)
check(math.abs(refresh_of(m) - 125) < 0.2, "no-stock 125 Hz refresh " .. refresh_of(m))

-- 8. An unwritable state file must never enable a trial.
write(state_path, "max=0\nmethod=clock\ntrial=135\ntrial_state=armed\n")
local real_open = io.open
io.open = function(path, mode)
    if path == state_path and mode == "w" then return nil end
    return real_open(path, mode)
end
load(true)
io.open = real_open
d, key = lookup(TMX)
check(key == "stock_lcd", "trial must stay off when its state cannot be recorded")

if failures > 0 then
    io.stderr:write(failures .. " check(s) failed\n")
    os.exit(1)
end
print("gamescope harness: all checks passed")
