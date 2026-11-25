local root = assert(PLUGIN_PATH)
_PLUGIN = { path = root }
local json = dofile(root .. "/json.lua")
local fixture = json.decode(assert(RECEIPT_FIXTURE))
local sdk = dofile(assert(TEST_ROOT) .. "/sdk.lua")
local e
local function bridge() return dofile(root .. "/Bridge.lua") end
local function request()
    return json.decode(e.fs[assert(e.directory) .. "/request.json"])
end
local function linked(project)
    e.a.fields.project, e.b.fields.project = project, project
end
local function publish(value, directory)
    value = value or json.decode(RECEIPT_FIXTURE)
    e.fs[value.project] = "project"
    for _, render in ipairs(value.renders) do e.fs[render.path] = "render" end
    e.fs[directory .. "/result.json"] = json.encode(value)
end
local function return_on_exit(value)
    e.onExit = function(directory) publish(value, directory) end
end
local function multi()
    local value = json.decode(RECEIPT_FIXTURE)
    value.renders[2] = { uuid = "a", source = "1", path = "/photos/one 'frame'-Antitone.tif" }
    return value
end
local function test(name, work)
    e = sdk.reset(fixture)
    work()
    print("PASS " .. name)
end

test("Extras exposes editing and explicit saved-project recovery without changing catalogue identifiers", function()
    local info = dofile(root .. "/Info.lua")
    assert(info.LrToolkitIdentifier == "com.antitone.lightroom.experiment")
    assert(info.LrPluginInfoProvider == "PluginInfo.lua")
    assert(#info.LrLibraryMenuItems == 2 and info.LrLibraryMenuItems[1].file == "Edit.lua")
    assert(info.LrLibraryMenuItems[2].file == "OpenSavedProject.lua")
    assert(dofile(root .. "/Metadata.lua").schemaVersion == 1)
    local config = dofile(root .. "/Config.lua")
    assert(config.read() == "/Applications/Antitone.app/Contents/MacOS/antitone")
    e.appPaths = {"/custom/Antitone's app.app"}
    e.fs[e.appPaths[1] .. "/Contents/MacOS/antitone"] = "binary"
    assert(config.choose() == e.appPaths[1] .. "/Contents/MacOS/antitone")
    assert(config.read() == e.prefs.executable)
    e.appPaths = {"/not-an-application"}
    assert(config.choose() == nil and config.read() == e.prefs.executable)
end)

test("Windows can choose its executable and launch both checks and editing with quoted space-containing paths", function()
    MAC_ENV, WIN_ENV = false, true
    local executable = "C:/Program Files/Antitone/antitone.exe"
    e.appPaths, e.tempPath = {executable}, "C:/Users/Test User/AppData/Local/Temp"
    e.fs[executable] = "windows binary"
    local value = json.decode(RECEIPT_FIXTURE)
    value.project = "D:/Photo Projects/roll.antitone"
    value.sources[1].path, value.sources[2].path = "D:/Raw Photos/one.ARW", "D:/Raw Photos/two.ARW"
    e.a.path, e.b.path = value.sources[1].path, value.sources[2].path
    e.fs[e.a.path], e.fs[e.b.path] = "raw-a", "raw-b"
    value.renders[1].path = "D:/Raw Photos/two-Antitone.tif"
    return_on_exit(value); bridge().edit()
    assert(e.prefs.executable == executable and e.addCalls == 1)
    assert(request().project == nil and request().photos[1].path == e.b.path)
    assert(#e.commands == 2)
    for _, command in ipairs(e.commands) do
        assert(command:find(' -Executable "' .. executable .. '"', 1, true))
        assert(command:find(' -Log "' .. e.directory, 1, true))
    end
    assert(not e.commands[1]:find(' -Request ', 1, true))
    assert(e.commands[2]:find(' -Request "' .. e.directory .. '/request.json"', 1, true))
end)

test("a blank protocol reply is diagnosed as a launch failure rather than an incompatible version", function()
    e.protocol = ""
    local ok, err = pcall(function() bridge().edit() end)
    assert(not ok and not e.launched)
    assert(tostring(err):find("did not report its Lightroom protocol version", 1, true))
    assert(not tostring(err):find("not compatible", 1, true))
end)

test("a numeric protocol mismatch still reports incompatible versions", function()
    e.protocol = "3\n"
    local ok, err = pcall(function() bridge().edit() end)
    assert(not ok and not e.launched)
    assert(tostring(err):find("not compatible", 1, true))
end)

test("existing development executable overrides remain usable without changing preferences", function()
    local original = dofile
    local override = root .. "/Config.local.lua"
    e.fs[override], e.fs["/custom/development/antitone"] = "configuration", "binary"
    dofile = function(path)
        if path == override then return { executable = "/custom/development/antitone" } end
        return original(path)
    end
    local ok, path = pcall(function() return dofile(root .. "/Config.lua").read() end)
    dofile = original
    assert(ok and path == "/custom/development/antitone" and e.prefs.executable == nil)
end)

test("Plugin Manager application chooser updates its bound setting through an SDK task", function()
    local provider, properties = dofile(root .. "/PluginInfo.lua"), {}
    provider.startDialog(properties)
    local f = {}
    function f:static_text(value) return value end
    function f:push_button(value) return value end
    local section = provider.sectionsForTopOfDialog(f, properties)[1]
    assert(section.bind_to_object == properties)
    section[3].action()
    assert(properties.application == e.prefs.executable)
end)

test("JSON request is active-first; no receipt imports nothing and creates no persistent session state", function()
    bridge().edit()
    assert(validateRequestJson(e.fs[e.directory .. "/request.json"]))
    assert(request().photos[1].uuid == "b" and request().photos[2].uuid == "a")
    assert(request().output == nil and e.addCalls == 0 and e.launched and not e.confirmCount)
    assert(e.directory:find("/tmp/Antitone-Lightroom", 1, true) == 1)
    assert(e.prefs.lastSession == nil and e.prefs.lastCatalog == nil)
    assert(e.fs[e.directory .. "/session.json"] == nil and e.fs[e.directory .. "/imported.txt"] == nil)
end)

test("old session preferences and files are ignored without importing or deleting them", function()
    e.prefs.lastSession, e.prefs.lastCatalog = "/old/return", e.catalogPath
    e.fs["/old/return/result.json"] = RECEIPT_FIXTURE
    bridge().edit()
    assert(e.addCalls == 0 and e.prefs.lastSession == "/old/return")
    assert(e.fs["/old/return/result.json"] == RECEIPT_FIXTURE)
end)

test("completed invocation automatically links the whole roll, preserves string IDs and stacks its results", function()
    return_on_exit(); bridge().edit()
    local returned = e.catalog:findPhotoByPath(fixture.renders[1].path)
    assert(e.a.fields.project == fixture.project and e.a.fields.source == "1")
    assert(e.b.fields.source == "18446744073709551615")
    assert(returned.fields.original == "b" and returned.fields.source == e.b.fields.source)
    assert(e.catalog.active == returned and e.stackCalls[1].stack == e.b and e.stackCalls[1].position == "above")
    assert(e.addCalls == 1)
end)

test("selecting original and returned TIFF deduplicates to its original and saved project", function()
    return_on_exit(); bridge().edit()
    local returned = e.catalog:findPhotoByPath(fixture.renders[1].path)
    e.catalog.selected, e.catalog.active = {e.b, returned}, returned
    e.onExit = nil; bridge().edit()
    assert(request().mode == "open" and request().project == fixture.project)
    assert(#request().photos == 1 and request().photos[1].uuid == "b" and request().photos[1].path == e.b.path)
end)

test("busy catalogue keeps saved files; retry opens the saved project and renders a fresh return", function()
    return_on_exit(); e.writeStatus = "aborted"
    local ok, err = pcall(function() bridge().edit() end)
    assert(not ok and tostring(err):find("catalogue is busy", 1, true))
    assert(e.addCalls == 0 and e.fs[fixture.project] == "project")
    local first = e.directory
    e.writeStatus = "executed"
    e.projectPaths = {fixture.project}
    local fresh = json.decode(RECEIPT_FIXTURE); fresh.renders[1].path = "/photos/second/return-2.tif"
    return_on_exit(fresh); bridge().openSavedProject()
    assert(e.directory ~= first and request().mode == "open" and e.addCalls == 1)
    assert(e.fs[fixture.renders[1].path] == "render" and e.fs[fresh.renders[1].path] == "render")
end)

test("rerendering after partial import preserves prior files and activates all new result folders", function()
    local value = multi()
    return_on_exit(value); e.failAddPath = value.renders[2].path
    assert(not pcall(function() bridge().edit() end))
    assert(e.addCalls == 1)
    local fresh = multi()
    fresh.renders[1].path, fresh.renders[2].path = "/photos/second/return-2.tif", "/photos/return-2.tif"
    e.failAddPath = nil; return_on_exit(fresh); bridge().edit()
    assert(request().mode == "open" and e.addCalls == 3 and #e.catalog.selected == 2)
    assert(#e.catalog.activeSources == 2)
    assert(e.stackCalls[2].stack == e.b and e.stackCalls[3].stack == e.a)
    assert(e.fs[value.renders[1].path] == "render" and e.fs[value.renders[2].path] == "render")
end)

test("an output already imported while the process was running is reused by path", function()
    e.onExit = function(directory)
        publish(nil, directory)
        e.catalog.photos[#e.catalog.photos + 1] = sdk.photo("already-imported", fixture.renders[1].path, "TIFF")
    end
    bridge().edit()
    assert(e.addCalls == 0 and e.catalog.active.uuid == "already-imported")
    assert(e.catalog.active.fields.original == "b")
end)

test("conflicting association is not silently reassigned during automatic import", function()
    e.catalog.selected, e.catalog.active = {e.b}, e.b
    e.a.fields.project = "/another/roll.antitone"
    return_on_exit()
    local ok, err = pcall(function() bridge().edit() end)
    assert(not ok and tostring(err):find("another Antitone project", 1, true))
    assert(e.addCalls == 0 and e.a.fields.project == "/another/roll.antitone")
end)

test("changing catalogues during the process cannot import into a different catalogue", function()
    e.onExit = function(directory)
        publish(nil, directory); e.catalogPath = "/catalog/other.lrcat"
    end
    assert(not pcall(function() bridge().edit() end))
    assert(e.addCalls == 0 and e.b.fields.project == nil)
end)

test("an original moved while editing is rejected before catalogue association or import", function()
    e.onExit = function(directory)
        publish(nil, directory); e.b.path = "/moved/frame.ARW"
    end
    local ok, err = pcall(function() bridge().edit() end)
    assert(not ok and tostring(err):find("Restore its original location", 1, true))
    assert(e.addCalls == 0 and e.a.fields.project == nil and e.b.fields.project == nil)
end)

test("cross-folder receipt imports without an invalid stack position", function()
    local value = json.decode(RECEIPT_FIXTURE)
    value.renders[1].path = "/renders/positive.tif"
    return_on_exit(value); bridge().edit()
    assert(e.stackCalls[1].stack == nil and e.stackCalls[1].position == nil)
end)

test("invalid JSON, versions, source ID types and empty returns fail before catalogue import", function()
    local invalid, numeric = json.decode(RECEIPT_FIXTURE), json.decode(RECEIPT_FIXTURE)
    invalid.version, numeric.sources[1].id = 2, 1
    for _, text in ipairs({"not JSON", json.encode(invalid), json.encode(numeric),
        '{"version":4,"project":"/photos/roll.antitone","sources":[],"renders":[]}'}) do
        e.onExit = function(directory) e.fs[directory .. "/result.json"] = text end
        assert(not pcall(function() bridge().edit() end))
        assert(e.addCalls == 0)
    end
end)

test("a missing configured application reopens the chooser without clearing the old selection on cancel", function()
    e.prefs.executable = "/missing/antitone"
    e.appPaths = nil
    local config = dofile(root .. "/Config.lua")
    assert(config.read() == nil and e.prefs.executable == "/missing/antitone")
    e.appPaths = {"/Applications/Antitone.app"}
    assert(config.read() == "/Applications/Antitone.app/Contents/MacOS/antitone")
end)

local function publish_save(directory)
    local value = json.decode(RECEIPT_FIXTURE)
    value.renders = nil
    e.fs[value.project] = "saved edits"
    e.fs[directory .. "/saved.json"] = json.encode(value)
end

test("ordinary save links the whole roll without importing or selecting TIFFs; Edit reopens it", function()
    e.onExit = function(directory)
        assert(request().mode == "new" and request().project == nil)
        publish_save(directory)
    end
    bridge().edit()
    assert(e.a.fields.project == fixture.project and e.b.fields.project == fixture.project)
    assert(e.a.fields.source == "1" and e.b.fields.source == "18446744073709551615")
    assert(e.addCalls == 0 and e.catalog.active == e.b and not e.catalog.activeSources)
    e.onExit = nil
    bridge().edit()
    assert(request().mode == "open" and request().project == fixture.project)
    assert(e.confirmCount == 1 and e.addCalls == 0)
end)

test("a completed save-back both links the saved project and imports its render", function()
    e.onExit = function(directory)
        publish_save(directory)
        publish(nil, directory)
    end
    bridge().edit()
    assert(e.a.fields.project == fixture.project and e.b.fields.project == fixture.project)
    assert(e.addCalls == 1 and e.catalog.active.path == fixture.renders[1].path)
end)

test("a failed render or nonzero exit does not lose the saved project association", function()
    e.onExit = function(directory)
        publish_save(directory)
        e.fs[directory .. "/result.json"] = "invalid return"
    end
    e.appStatus = 1
    assert(not pcall(function() bridge().edit() end))
    assert(e.a.fields.project == fixture.project and e.b.fields.project == fixture.project)
    assert(e.addCalls == 0)
end)

test("an existing linked project can be declined without a chooser or launch", function()
    e.fs[fixture.project] = "saved edits"
    linked(fixture.project); e.confirms = {"cancel"}
    bridge().edit()
    assert(not e.launched and not e.directory and e.confirmCount == 1)
end)

test("missing linked project can be cancelled without clearing metadata", function()
    linked("/missing/roll.antitone"); e.confirms = {"cancel"}
    bridge().edit()
    assert(not e.launched and e.a.fields.project == "/missing/roll.antitone")
end)

test("locating a missing project changes associations only on return; older TIFFs follow the original", function()
    local missing, moved = "/missing/roll.antitone", "/moved/roll.antitone"
    linked(missing); e.fs[moved] = "saved-edits"; e.projectPaths = {moved}
    local oldReturn = sdk.photo("old-return", "/old/return.tif", "TIFF")
    oldReturn.fields.project, oldReturn.fields.original = missing, "b"
    e.catalog.photos[#e.catalog.photos + 1] = oldReturn
    bridge().edit()
    assert(request().project == moved and request().mode == "open" and request().relocated_from == missing)
    assert(e.a.fields.project == missing and e.b.fields.project == missing)
    local value = json.decode(RECEIPT_FIXTURE); value.project = moved
    return_on_exit(value); bridge().edit()
    assert(e.a.fields.project == moved and e.b.fields.project == moved)
    e.catalog.selected, e.catalog.active = {oldReturn}, oldReturn
    local prompts = e.confirmCount
    e.onExit = nil; bridge().edit()
    assert(request().project == moved and e.confirmCount == prompts + 1)
end)

test("explicit replacement creates a fresh selected roll and defers relinking until return", function()
    local missing, fresh = "/missing/roll.antitone", "/projects/replacement.antitone"
    linked(missing); e.confirms = {"other"}
    local value = json.decode(RECEIPT_FIXTURE); value.project = fresh
    e.onExit = function(directory)
        assert(request().mode == "new" and request().project == nil and e.a.fields.project == missing)
        publish(value, directory)
    end
    bridge().edit()
    assert(e.a.fields.project == fresh and e.b.fields.project == fresh)
end)

test("protocol probe failure offers the current diagnostic log before launching an editor", function()
    e.protocolStatus, e.protocol = 2, "unsupported option\n"
    dofile(root .. "/Run.lua")("edit")
    assert(not e.launched and e.revealed and e.fs[e.revealed] == "unsupported option\n")
end)

test("failed imports offer their log and rerender instructions without Lua source locations", function()
    return_on_exit(); e.writeStatus = "aborted"
    e.confirms = {"ok", "ok"}
    dofile(root .. "/Run.lua")("edit")
    assert(e.revealed == e.directory .. "/session.log")
    local failure = e.confirmations[#e.confirmations]
    assert(failure[2]:find("Open Saved Antitone Project", 1, true) and failure[3] == "Show Log")
    assert(e.fs[e.revealed] == "application diagnostic" and e.fs[fixture.project] == "project")
    e.catalog.active = nil
    dofile(root .. "/Run.lua")("edit")
    local message = e.messages[#e.messages]
    assert(message[3] == "critical" and message[2]:find("Select one or more photos", 1, true))
    assert(message[2]:find("Open Saved Antitone Project", 1, true) and not message[2]:find("Bridge.lua", 1, true))
end)

test("SDK metadata can yield during active-first multi-photo opening", function()
    local c = sdk.photo("c", "/photos/third.ARW")
    e.fs[c.path] = "raw-c"; e.catalog.photos[#e.catalog.photos + 1] = c
    e.catalog.selected, e.catalog.active = {e.a, e.b, c}, c
    e.yieldMetadata = true
    local task = coroutine.create(function() bridge().edit() end)
    local yields = 0
    repeat
        local ok, why = coroutine.resume(task); assert(ok, why)
        if coroutine.status(task) ~= "dead" then yields = yields + 1 end
    until coroutine.status(task) == "dead"
    assert(yields > 0 and request().photos[1].uuid == "c" and request().photos[2].uuid == "a")
end)
