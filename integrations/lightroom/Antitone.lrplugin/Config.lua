local LrDialogs = import "LrDialogs"
local LrFileUtils = import "LrFileUtils"
local LrPathUtils = import "LrPathUtils"
local LrPrefs = import "LrPrefs"

local prefs = LrPrefs.prefsForPlugin()
local config = {}

local function executable(path)
    if path and path:lower():match("%.app/?$") then
        return LrPathUtils.child(path, "Contents/MacOS/antitone")
    end
    return path
end

function config.choose()
    local paths = LrDialogs.runOpenPanel {
        title = "Choose Antitone application",
        prompt = "Choose Antitone",
        canChooseFiles = true,
        canChooseDirectories = true,
        allowsMultipleSelection = false,
    }
    if not paths then return nil end
    local path = executable(paths[1])
    if not path or LrFileUtils.exists(path) ~= "file" then
        LrDialogs.message("Not an Antitone application",
            "Choose the Antitone application or its executable. The previous setting has been kept.", "warning")
        return nil
    end
    prefs.executable = path
    return path
end

function config.read()
    if prefs.executable then
        if LrFileUtils.exists(prefs.executable) == "file" then return prefs.executable end
        LrDialogs.message("Antitone application is missing", prefs.executable .. "\nChoose its new location or another installation.", "warning")
        return config.choose()
    end
    -- Preserve existing development installations without making Lua editing the setup path.
    local override = LrPathUtils.child(_PLUGIN.path, "Config.local.lua")
    if LrFileUtils.exists(override) == "file" then
        local path = executable(dofile(override).executable)
        if path and LrFileUtils.exists(path) == "file" then return path end
    end
    if not WIN_ENV then
        local path = "/Applications/Antitone.app/Contents/MacOS/antitone"
        if LrFileUtils.exists(path) == "file" then return path end
    end
    return config.choose()
end

return config
