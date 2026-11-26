local LrDialogs = import "LrDialogs"
local LrFileUtils = import "LrFileUtils"
local projects = {}

function projects.existing()
    local paths = LrDialogs.runOpenPanel {
        title = "Choose saved Antitone project", fileTypes = { "antitone", "json" },
        allowsMultipleSelection = false,
    }
    if not paths then return nil end
    if LrFileUtils.exists(paths[1]) == "file" then return paths[1] end
    LrDialogs.message("Project is unavailable", paths[1], "warning")
    return nil
end

function projects.choose(linked, chooseSaved)
    if not linked then
        if not chooseSaved then return "new" end
        local path = projects.existing()
        if path then return "open", path end
        return nil
    end
    if LrFileUtils.exists(linked) == "file" then
        if LrDialogs.confirm("Open saved Antitone project?", linked, "Open Project", "Cancel") == "ok" then
            return "open", linked
        end
        return nil
    end
    local choice = LrDialogs.confirm("Linked Antitone project is missing",
        linked .. "\n\nLocate the saved project if it moved, or start a replacement from the selected originals. A replacement cannot recover edits from the missing project.",
        "Locate Saved Project", "Cancel", "Create Replacement Project")
    if choice == "other" then return "new", nil, linked end
    if choice == "ok" then
        local path = projects.existing()
        if path then return "open", path, linked end
    end
end

return projects
