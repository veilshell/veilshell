-- Optional Omarchy 4.5+ Lua Hyprland snippet for VeilShell
-- Require or dofile from ~/.config/hypr/hyprland.lua (never modify /usr/share/omarchy/).
--
-- Example:
--   dofile(os.getenv("HOME") .. "/Projects/veilshell/adapters/omarchy/veilshell.lua")

-- Keep VeilShell surfaces crisp without Hyprland fade animations fighting spring physics
hl.layer_rule({ match = { namespace = "veilshell" }, no_anim = true, animation = "none" })

-- Optional: launch the VeilShell pill daemon on startup (or toggle via keybind below)
-- hl.exec_cmd("veilshell --scene pill")

-- Optional touch/tablet bindings using Omarchy helpers:
-- o.bind("SUPER + SHIFT + T", "Toggle VeilShell Tablet Mode", "veilctl mode toggle")
-- o.bind("SUPER + SHIFT + H", "Show VeilShell Home Grid", "veilctl show home")
-- o.bind("SUPER + SHIFT + R", "Show VeilShell Recents Deck", "veilctl show recents")
-- o.bind("SUPER + SHIFT + Q", "Show VeilShell Quick Settings", "veilctl show shade")
