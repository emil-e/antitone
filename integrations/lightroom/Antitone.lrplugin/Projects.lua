local LrDialogs = import "LrDialogs"
local LrFileUtils = import "LrFileUtils"
local projects = {}

local function existing()
    local paths = LrDialogs.runOpenPanel {
        title = "Choose saved Antitone roll", fileTypes = { "antitone", "json" },
        allowsMultipleSelection = false,
    }
    if not paths then return nil end
    if LrFileUtils.exists(paths[1]) == "file" then return paths[1] end
    LrDialogs.message("Project is unavailable", paths[1], "warning")
    return nil
end

local function create()
    while true do
        local path = LrDialogs.runSavePanel {
            title = "Save Antitone roll", requiredFileType = "antitone", canCreateDirectories = true,
        }
        if not path then return nil end
        if not LrFileUtils.exists(path) then return path end
        LrDialogs.message("Choose a different project filename",
            path .. " already exists. It has not been changed. Choose an unused filename, or cancel and use a saved roll.", "warning")
    end
end

function projects.choose(linked)
    if linked and LrFileUtils.exists(linked) == "file" then return linked, "open", nil end
    if linked then
        local choice = LrDialogs.confirm("Linked Antitone project is missing",
            linked .. "\n\nLocate the saved roll if it moved. Alternatively, create a replacement roll from the selected originals; this cannot recover edits from the missing project. Catalogue links change only after a successful return.",
            "Locate Saved Roll", "Cancel", "Create Replacement Roll")
        if choice == "cancel" then return nil end
        if choice == "other" then return create(), "new", linked end
        return existing(), "open", linked
    end
    local choice = LrDialogs.confirm("Choose an Antitone roll",
        "Create a roll from the selected originals, or link them to an existing saved roll. Lightroom development is not applied.",
        "Create New Roll", "Cancel", "Use Saved Roll")
    if choice == "cancel" then return nil end
    if choice == "other" then return existing(), "open", nil end
    return create(), "new", nil
end

return projects
