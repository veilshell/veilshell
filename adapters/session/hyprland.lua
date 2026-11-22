-- Standalone session; do not source the user's regular Hyprland config.
hl.monitor({ output = "", mode = "preferred", position = "auto", scale = "auto" })
hl.config({
    misc = {
        disable_hyprland_logo = true,
        disable_splash_rendering = true,
    },
})

hl.layer_rule({ match = { namespace = "veilshell" }, no_anim = true })

hl.on("hyprland.start", function()
    -- This command inherits the new compositor's Wayland and IPC endpoints.
    hl.exec_cmd("veilshell-session --publish-display")
end)

hl.bind("SUPER + H", hl.dsp.exec_cmd("veilctl show home"))
hl.bind("SUPER + R", hl.dsp.exec_cmd("veilctl show recents"))
hl.bind("SUPER + S", hl.dsp.exec_cmd("veilctl show shade"))
hl.bind("SUPER + L", hl.dsp.exec_cmd("veilshell-session --lock"))
hl.bind("SUPER + SHIFT + E", hl.dsp.exec_cmd("veilctl quit"))
