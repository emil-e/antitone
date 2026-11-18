local sdk = {}
local active

local function photo(uuid, path, format)
    local p = { uuid = uuid, path = path, format = format or "RAW", fields = {} }
    function p:getRawMetadata(key)
        if active.yieldMetadata then coroutine.yield("metadata read") end
        if key == "uuid" then return self.uuid end
        if key == "path" then return self.path end
        if key == "fileFormat" then return self.format end
        error("Unexpected metadata key " .. key)
    end
    function p:getPropertyForPlugin(_, key) return self.fields[key] end
    function p:setPropertyForPlugin(_, key, value) self.fields[key] = value end
    return p
end
sdk.photo = photo

function sdk.reset(fixture)
    MAC_ENV, WIN_ENV = true, false
    local e = {
        fs = {}, prefs = {}, messages = {}, commands = {}, stackCalls = {}, addCalls = 0,
        writeStatus = "executed", confirms = {}, confirmations = {}, savePaths = {fixture.project},
        appPaths = {"/Applications/Antitone.app"}, catalogPath = "/catalog/test.lrcat",
        protocol = "3\n", protocolStatus = 0, appStatus = 0,
    }
    active = e
    e.a, e.b = photo("a", fixture.sources[1].path), photo("b", fixture.sources[2].path)
    e.fs[e.a.path], e.fs[e.b.path] = "raw-a", "raw-b"
    e.fs["/Applications/Antitone.app/Contents/MacOS/antitone"] = "binary"
    local c = { photos = {e.a, e.b}, selected = {e.a, e.b}, active = e.b }
    e.catalog = c
    function c:getPath() return e.catalogPath end
    function c:getTargetPhoto() return self.active end
    function c:getTargetPhotos()
        local result = {}; for i, p in ipairs(self.selected) do result[i] = p end; return result
    end
    function c:findPhotoByUuid(uuid) for _, p in ipairs(self.photos) do if p.uuid == uuid then return p end end end
    function c:findPhotoByPath(path) for _, p in ipairs(self.photos) do if p.path == path then return p end end end
    function c:getFolderByPath(path) return {path = path} end
    function c:setActiveSources(sources) self.activeSources = sources end
    function c:setSelectedPhotos(selected, others)
        self.active, self.selected = selected, {selected}
        for _, p in ipairs(others) do self.selected[#self.selected + 1] = p end
        for _, p in ipairs(self.selected) do
            local visible = false
            for _, folder in ipairs(self.activeSources) do
                if p.path:match("^(.*)/[^/]+$") == folder.path then visible = true end
            end
            assert(visible, "returned selection must be visible in active folders")
        end
    end
    function c:withWriteAccessDo(_, work)
        if e.writeStatus == "executed" then work() end
        return e.writeStatus
    end
    function c:addPhoto(path, stack, position)
        assert(stack or position == nil, "stack position requires a target")
        if path == e.failAddPath then error("simulated catalogue import failure") end
        e.addCalls = e.addCalls + 1
        e.stackCalls[#e.stackCalls + 1] = {stack = stack, position = position}
        local p = photo("returned-" .. e.addCalls, path, "TIFF")
        self.photos[#self.photos + 1] = p
        return p
    end
    local modules = {
        LrApplication = { activeCatalog = function() return c end },
        LrPrefs = { prefsForPlugin = function() return e.prefs end },
        LrPathUtils = {
            child = function(parent, name) return parent .. "/" .. name end,
            parent = function(path) return path:match("^(.*)/[^/]+$") end,
            getStandardFilePath = function(kind) assert(kind == "temp"); return e.tempPath or "/tmp" end,
        },
        LrFileUtils = {
            exists = function(path) return e.fs[path] and (e.fs[path] == true and "directory" or "file") or false end,
            readFile = function(path) return assert(e.fs[path], "Missing mock file " .. path) end,
            createDirectory = function(path) assert(not e.fs[path]); e.fs[path] = true end,
            chooseUniqueFileName = function(path)
                local i, base = 2, path
                while e.fs[path] do path = base .. "-" .. i; i = i + 1 end
                return path
            end,
        },
        LrDialogs = {
            confirm = function(...)
                e.confirmCount = (e.confirmCount or 0) + 1
                e.confirmations[#e.confirmations + 1] = {...}
                return table.remove(e.confirms, 1) or "ok"
            end,
            runSavePanel = function() return table.remove(e.savePaths, 1) end,
            runOpenPanel = function(args)
                if args.title == "Choose Antitone application" then return e.appPaths end
                return e.projectPaths
            end,
            message = function(...) e.messages[#e.messages + 1] = {...} end,
        },
        LrTasks = {
            pcall = pcall,
            startAsyncTask = function(work) work() end,
            execute = function(command)
                e.commands[#e.commands + 1] = command
                if WIN_ENV then
                    assert(command:sub(1, 2) == '""' and command:sub(-1) == '"', "Windows command needs outer quotes")
                    command = command:sub(2, -2)
                end
                if command:find(" --lightroom-protocol-version ", 1, true) then
                    local path = assert(command:match(WIN_ENV and ' > "(.-)" 2>&1$' or " > '(.-)' 2>&1$"))
                    e.fs[path] = e.protocol
                    return e.protocolStatus
                end
                assert(command:find(" --lightroom ", 1, true))
                e.launched = command
                local path = assert(command:match(WIN_ENV and ' %-%-lightroom "(.-)" > ' or " %-%-lightroom '(.-)' > "))
                e.directory = assert(path:match("^(.*)/[^/]+$"))
                e.fs[e.directory .. "/session.log"] = "application diagnostic"
                if e.onExit then e.onExit(e.directory) end
                return e.appStatus
            end,
        },
        LrShell = { revealInShell = function(path) e.revealed = path end },
        LrView = { bind = function(key) return key end },
    }
    function import(name) return assert(modules[name], "Unexpected SDK import " .. name) end
    io.open = function(path)
        local contents = ""
        return {
            write = function(_, value) contents = contents .. value; return true end,
            close = function() e.fs[path] = contents; return true end,
        }
    end
    return e
end

return sdk
