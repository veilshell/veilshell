-- Standalone session; do not source the user's regular Hyprland config.
hl.monitor({ output = "", mode = "preferred", position = "auto", scale = "auto" })
hl.config({
    -- GSettings is shared by the user's other desktops.
    cursor = { sync_gsettings_theme = false },
    input = {
        touchpad = {
            natural_scroll = true,
            clickfinger_behavior = true,
            scroll_factor = 0.4,
        },
    },
    misc = {
        disable_hyprland_logo = true,
        disable_splash_rendering = true,
        key_press_enables_dpms = true,
        mouse_move_enables_dpms = true,
    },
})

hl.layer_rule({ match = { namespace = "veilshell" }, no_anim = true })
hl.bind("SUPER + CTRL + ALT + I", hl.dsp.exec_cmd("veilctl show settings"), { description = "VeilShell Settings" })

hl.on("hyprland.start", function()
    -- This command inherits the new compositor's Wayland and IPC endpoints.
    hl.exec_cmd("veilshell-session --publish-display")
end)

local session_config = os.getenv("VEILSHELL_SESSION_CONFIG_DIR") or "/usr/share/veilshell/session"
local xdg_config = os.getenv("XDG_CONFIG_HOME")
local user_config = ((xdg_config and xdg_config:sub(1, 1) == "/" and xdg_config) or (os.getenv("HOME") .. "/.config")) .. "/veilshell"
local profile = "preview"
local selection = io.open(user_config .. "/bindings.profile", "r")
if selection then
    profile = selection:read("*l")
    selection:close()
end

if profile == "omarchy" then
    dofile(session_config .. "/bindings-omarchy.lua")
elseif profile == "preview" then
    hl.bind("SUPER + H", hl.dsp.exec_cmd("veilctl show home"))
    hl.bind("SUPER + R", hl.dsp.exec_cmd("veilctl show recents"))
    hl.bind("SUPER + S", hl.dsp.exec_cmd("veilctl show shade"))
    hl.bind("SUPER + L", hl.dsp.exec_cmd("veilshell-session --lock"))
    hl.bind("SUPER + SHIFT + E", hl.dsp.exec_cmd("veilctl quit"))
else
    error("VeilShell bindings.profile must be 'preview' or 'omarchy'")
end

-- Explicitly opted-in standalone Lua, never the user's regular Hyprland tree.
local overrides = io.open(user_config .. "/bindings.lua", "r")
if overrides then
    overrides:close()
    dofile(user_config .. "/bindings.lua")
end
