-- Familiar chords, using only VeilShell, Hyprland and generic host commands.
local function command(keys, description, cmd, options)
    options = options or {}
    options.description = description
    hl.bind(keys, hl.dsp.exec_cmd(cmd), options)
end

command("SUPER + SPACE", "VeilShell Home", "veilctl show home")
command("SUPER + CTRL + ALT + R", "VeilShell Recents", "veilctl show recents")
command("SUPER + CTRL + ALT + S", "VeilShell Quick Settings", "veilctl show shade")
command("SUPER + CTRL + L", "Lock session", "veilshell-session --lock")
command("SUPER + RETURN", "Terminal", "veilshell-session --launch terminal")
command("SUPER + SHIFT + RETURN", "Default browser", "veilshell-session --launch browser")
command("SUPER + SHIFT + B", "Default browser", "veilshell-session --launch browser")
command("SUPER + SHIFT + F", "File manager", "veilshell-session --launch files")

hl.bind("SUPER + W", hl.dsp.window.close(), { description = "Close window" })
hl.bind("SUPER + Q", hl.dsp.window.close(), { description = "Close window" })
hl.bind("SUPER + F", hl.dsp.window.fullscreen({ mode = "fullscreen" }), { description = "Fullscreen" })
hl.bind("SUPER + T", hl.dsp.window.float({ action = "toggle" }), { description = "Toggle floating" })
hl.bind("ALT + TAB", hl.dsp.window.cycle_next(), { description = "Next window" })
hl.bind("ALT + SHIFT + TAB", hl.dsp.window.cycle_next({ next = false }), { description = "Previous window" })
hl.bind("SUPER + TAB", hl.dsp.focus({ workspace = "m+1" }), { description = "Next workspace" })
hl.bind("SUPER + SHIFT + TAB", hl.dsp.focus({ workspace = "m-1" }), { description = "Previous workspace" })
hl.bind("SUPER + S", hl.dsp.workspace.toggle_special("scratchpad"), { description = "Toggle scratchpad" })
hl.bind("SUPER + ALT + S", hl.dsp.window.move({ workspace = "special:scratchpad", follow = false }),
    { description = "Move to scratchpad" })

for _, pair in ipairs({ { "LEFT", "l" }, { "RIGHT", "r" }, { "UP", "u" }, { "DOWN", "d" } }) do
    hl.bind("SUPER + " .. pair[1], hl.dsp.focus({ direction = pair[2] }), { description = "Focus " .. pair[1] })
    hl.bind("SUPER + SHIFT + " .. pair[1], hl.dsp.window.swap({ direction = pair[2] }),
        { description = "Swap " .. pair[1] })
end

for workspace = 1, 10 do
    local key = "code:" .. (workspace + 9)
    local target = tostring(workspace)
    hl.bind("SUPER + " .. key, hl.dsp.focus({ workspace = target }), { description = "Go to workspace " .. target })
    hl.bind("SUPER + SHIFT + " .. key, hl.dsp.window.move({ workspace = target, follow = true }),
        { description = "Move to workspace " .. target })
    hl.bind("SUPER + SHIFT + ALT + " .. key, hl.dsp.window.move({ workspace = target, follow = false }),
        { description = "Send to workspace " .. target })
end

command("XF86AudioRaiseVolume", "Volume up", "wpctl set-volume -l 1.0 @DEFAULT_AUDIO_SINK@ 5%+",
    { repeating = true, locked = true })
command("XF86AudioLowerVolume", "Volume down", "wpctl set-volume @DEFAULT_AUDIO_SINK@ 5%-",
    { repeating = true, locked = true })
command("XF86AudioMute", "Mute", "wpctl set-mute @DEFAULT_AUDIO_SINK@ toggle", { locked = true })
command("XF86MonBrightnessUp", "Brightness up", "brightnessctl set +5%", { repeating = true, locked = true })
command("XF86MonBrightnessDown", "Brightness down", "brightnessctl set 5%-", { repeating = true, locked = true })
command("XF86AudioPlay", "Play/pause", "playerctl play-pause", { locked = true })
command("XF86AudioPause", "Play/pause", "playerctl play-pause", { locked = true })
command("XF86AudioNext", "Next track", "playerctl next", { locked = true })
command("XF86AudioPrev", "Previous track", "playerctl previous", { locked = true })
