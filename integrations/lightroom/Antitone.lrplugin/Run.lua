local LrTasks = import "LrTasks"
local LrDialogs = import "LrDialogs"
local LrFileUtils = import "LrFileUtils"
local LrShell = import "LrShell"

return function(action, completed)
    LrTasks.startAsyncTask(function()
        local bridge
        local ok, err = LrTasks.pcall(function()
            bridge = dofile(_PLUGIN.path .. "/Bridge.lua")
            local result = bridge[action]()
            if completed then completed(result) end
        end)
        if not ok then
            local message = tostring(err):gsub("^.-:%d+:%s*", "")
            if action == "edit" then
                message = message .. "\n\nIf Antitone saved a roll, choose Edit in Antitone again to reopen it and render again. If the originals are not linked yet, choose Use Saved Roll. Earlier TIFFs may remain on disk."
                if bridge and bridge.project then message = message .. "\nProject: " .. bridge.project end
            end
            local log = bridge and bridge.log
            if log and LrFileUtils.exists(log) == "file" then
                local choice = LrDialogs.confirm("Could not complete Antitone action",
                    message .. "\n\nLog: " .. log, "Show Log", "Close")
                if choice == "ok" then
                    local shown, reason = LrTasks.pcall(function() LrShell.revealInShell(log) end)
                    if not shown then LrDialogs.message("Could not show log", log .. "\n" .. tostring(reason), "critical") end
                end
            else
                LrDialogs.message("Could not complete Antitone action", message, "critical")
            end
        end
    end)
end
