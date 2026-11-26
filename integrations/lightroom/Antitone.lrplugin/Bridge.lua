local LrApplication = import "LrApplication"
local LrDialogs = import "LrDialogs"
local LrFileUtils = import "LrFileUtils"
local LrPathUtils = import "LrPathUtils"
local LrTasks = import "LrTasks"
local json = dofile(_PLUGIN.path .. "/json.lua")
local protocol_version = dofile(_PLUGIN.path .. "/Protocol.lua")

local projects = dofile(_PLUGIN.path .. "/Projects.lua")
local config = dofile(_PLUGIN.path .. "/Config.lua")
local bridge = {}

local function child(parent, name) return LrPathUtils.child(parent, name) end
local function quote(value)
    if WIN_ENV then return '"' .. value .. '"' end
    return "'" .. value:gsub("'", "'\\''") .. "'"
end
local function launch(executable, log, request)
    if WIN_ENV then
        local command = "powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "
            .. quote(child(_PLUGIN.path, "Launch.ps1")) .. " -Executable " .. quote(executable)
            .. " -Log " .. quote(log)
        if request then command = command .. " -Request " .. quote(request) end
        return LrTasks.execute(command)
    end
    local arguments = request and ("--lightroom " .. quote(request)) or "--lightroom-protocol-version"
    return LrTasks.execute(quote(executable) .. " " .. arguments .. " > " .. quote(log) .. " 2>&1")
end
local function write(path, contents)
    local file, err = io.open(path, "wb")
    assert(file, err)
    local ok, message = file:write(contents)
    local closed, closeError = file:close()
    assert(ok, message)
    assert(closed, closeError)
end

local function array(value)
    assert(type(value) == "table" and #value > 0, "Missing return entries")
    local count = 0
    for key in pairs(value) do
        assert(type(key) == "number" and key % 1 == 0 and key >= 1 and key <= #value,
            "Invalid return array")
        count = count + 1
    end
    assert(count == #value, "Incomplete return array")
    return value
end

local function receipt(path, returning)
    local wire = json.decode(LrFileUtils.readFile(path))
    assert(type(wire) == "table" and wire.version == protocol_version, "Unsupported Antitone receipt")
    assert(type(wire.project) == "string" and #wire.project > 0, "Missing Antitone project")
    local result = { project = wire.project, sources = {} }
    if returning then
        result.renders = array(wire.renders)
    else
        assert(wire.renders == nil, "Unexpected renders in project save receipt")
    end
    for _, source in ipairs(array(wire.sources)) do
        assert(type(source.id) == "string" and source.id:match("^%d+$"), "Invalid Antitone source ID")
        assert(type(source.path) == "string", "Invalid Antitone source path")
        assert(not result.sources[source.id], "Duplicate Antitone source")
        result.sources[source.id] = source.path
    end
    for _, render in ipairs(result.renders or {}) do
        assert(type(render.uuid) == "string" and #render.uuid > 0, "Invalid Lightroom UUID")
        assert(type(render.source) == "string" and result.sources[render.source],
            "Antitone receipt references a missing source")
        assert(type(render.path) == "string", "Invalid rendered path")
        assert(LrFileUtils.exists(render.path) == "file", "Returned image is missing: " .. render.path)
    end
    assert(LrFileUtils.exists(result.project) == "file", "Saved Antitone project is missing")
    return result
end

local function associate(photo, project, source, original)
    photo:setPropertyForPlugin(_PLUGIN, "project", project)
    photo:setPropertyForPlugin(_PLUGIN, "source", source)
    photo:setPropertyForPlugin(_PLUGIN, "original", original)
end

local function original_at(catalog, uuid, path)
    local original = catalog:findPhotoByUuid(uuid)
    assert(original, "The original photo is no longer in this catalogue")
    assert(original:getRawMetadata("path") == path,
        "The original moved during editing. Restore its original location, then reopen the saved project.")
    return original
end

local function finish(catalog, result, replacedProject)
    -- A catalogue write never spans the external editing session.
    local originals = {}
    for _, render in ipairs(result.renders or {}) do
        originals[render.uuid] = original_at(catalog, render.uuid, result.sources[render.source])
    end
    local returned = {}
    local status = catalog:withWriteAccessDo("Link Antitone project", function()
        for source, path in pairs(result.sources) do
            local original = catalog:findPhotoByPath(path)
            if original then
                local previous = original:getPropertyForPlugin(_PLUGIN, "project")
                assert(not previous or previous == result.project or previous == replacedProject,
                    "This photo is already associated with another Antitone project: " .. path)
                associate(original, result.project, source, original:getRawMetadata("uuid"))
            end
        end
        for _, render in ipairs(result.renders or {}) do
            local photo = catalog:findPhotoByPath(render.path)
            if not photo then
                local original = originals[render.uuid]
                local sameFolder = LrPathUtils.parent(render.path) == LrPathUtils.parent(original:getRawMetadata("path"))
                if sameFolder then
                    photo = catalog:addPhoto(render.path, original, "above")
                else
                    photo = catalog:addPhoto(render.path)
                end
            end
            assert(photo, "Lightroom did not import " .. render.path)
            associate(photo, result.project, render.source, render.uuid)
            returned[#returned + 1] = photo
        end
    end, { timeout = 10 })
    assert(status == "executed", "Lightroom catalogue is busy. Reopen the saved Antitone project to try again.")
    if not result.renders then return end
    local folders, seen = {}, {}
    for _, render in ipairs(result.renders) do
        local path = LrPathUtils.parent(render.path)
        if not seen[path] then
            seen[path] = true
            local folder = catalog:getFolderByPath(path)
            if folder then folders[#folders + 1] = folder end
        end
    end
    if #folders > 0 then catalog:setActiveSources(folders) end
    local others = {}
    for i = 2, #returned do others[#others + 1] = returned[i] end
    catalog:setSelectedPhotos(returned[1], others)
    LrDialogs.message("Returned from Antitone", tostring(#result.renders)
        .. " image(s) imported. All catalogue photos in the roll are linked to the project.", "info")
end

function bridge.configure() return config.choose() end

local function edit(chooseSaved)
    local catalog = LrApplication.activeCatalog()
    local catalogPath = catalog:getPath()
    assert(catalog:getTargetPhoto(), "Select one or more photos first")
    local selected = catalog:getTargetPhotos()
    local active = catalog:getTargetPhoto()
    -- SDK metadata reads can yield; keep them out of C sort callbacks.
    for index, photo in ipairs(selected) do
        if photo == active then
            table.remove(selected, index)
            table.insert(selected, 1, photo)
            break
        end
    end
    local project, photos, seen = nil, {}, {}
    for _, selectedPhoto in ipairs(selected) do
        local originalUuid = selectedPhoto:getPropertyForPlugin(_PLUGIN, "original")
        local original = selectedPhoto
        if originalUuid then original = catalog:findPhotoByUuid(originalUuid) end
        assert(original, "The linked original is no longer in this catalogue")
        -- The original owns the current association; older returned TIFFs can carry a stale locator.
        local linked = original:getPropertyForPlugin(_PLUGIN, "project")
            or selectedPhoto:getPropertyForPlugin(_PLUGIN, "project")
        if linked then
            assert(not project or project == linked,
                "Selected photos belong to different Antitone projects; open one project at a time")
            project = linked
        end
        assert(original:getRawMetadata("fileFormat") == "RAW" or original:getRawMetadata("fileFormat") == "DNG",
            "Select camera RAW originals or TIFFs returned by Antitone")
        local uuid = original:getRawMetadata("uuid")
        if not seen[uuid] then
            local path = original:getRawMetadata("path")
            assert(LrFileUtils.exists(path) == "file", "Original RAW is offline: " .. path)
            photos[#photos + 1] = { uuid = uuid, path = path }
            seen[uuid] = true
        end
    end
    local mode, chosen, replacedProject = projects.choose(project, chooseSaved)
    if not mode then return end
    project = chosen
    bridge.project = project
    local executable = config.read()
    if not executable then return end
    local request = { version = protocol_version, mode = mode, project = project, photos = photos }
    if mode == "open" then request.relocated_from = replacedProject end
    local directory = LrFileUtils.chooseUniqueFileName(child(LrPathUtils.getStandardFilePath("temp"), "Antitone-Lightroom"))
    LrFileUtils.createDirectory(directory)
    assert(LrFileUtils.exists(directory) == "directory", "Cannot create temporary return folder: " .. directory)
    local requestPath = child(directory, "request.json")
    write(requestPath, json.encode(request))
    local protocolPath = child(directory, "protocol.txt")
    bridge.log = protocolPath
    local protocolStatus = launch(executable, protocolPath)
    local protocol = LrFileUtils.exists(protocolPath) == "file" and LrFileUtils.readFile(protocolPath)
    local version = protocol and protocol:match("^%s*(%d+)%s*$")
    assert(protocolStatus == 0 and version,
        "Antitone did not report its Lightroom protocol version (exit status " .. tostring(protocolStatus)
            .. "). Check the application path in Plug-in Manager. Details: " .. protocolPath)
    assert(version == tostring(protocol_version),
        "This Antitone application is not compatible with the plug-in. Install a matching app/plug-in pair, then choose the application in Plug-in Manager. Details: " .. protocolPath)
    bridge.log = child(directory, "session.log")
    -- The SDK task waits for this process without freezing Lightroom's UI.
    local status = launch(executable, bridge.log, requestPath)
    local savedPath, resultPath = child(directory, "saved.json"), child(directory, "result.json")
    local saved = LrFileUtils.exists(savedPath) == "file"
    local returned = LrFileUtils.exists(resultPath) == "file"
    if saved or returned then
        assert(catalog:getPath() == catalogPath and LrApplication.activeCatalog():getPath() == catalogPath,
            "The originating catalogue is no longer active. Reopen the saved project from that catalogue.")
    end
    if saved then
        local result = receipt(savedPath, false)
        bridge.project = result.project
        for _, photo in ipairs(photos) do original_at(catalog, photo.uuid, photo.path) end
        finish(catalog, result, replacedProject)
    end
    if returned then
        local result = receipt(resultPath, true)
        assert(not saved or result.project == bridge.project, "Return does not match the saved project")
        finish(catalog, result, replacedProject)
    elseif status ~= 0 then
        error("Antitone exited without a completed return (status " .. tostring(status)
            .. "). See " .. child(directory, "session.log"))
    end
end

function bridge.edit() return edit(false) end
function bridge.openSavedProject() return edit(true) end

return bridge
