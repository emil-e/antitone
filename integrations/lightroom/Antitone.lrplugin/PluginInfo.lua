local LrPrefs = import "LrPrefs"
local LrView = import "LrView"

return {
    startDialog = function(properties)
        properties.application = LrPrefs.prefsForPlugin().executable or "Choose the Antitone application before the first edit"
    end,
    sectionsForTopOfDialog = function(f, properties)
        return {
            {
                title = "Antitone",
                bind_to_object = properties,
                f:static_text { title = "Application used to edit RAWs and open Antitone projects:" },
                f:static_text { title = LrView.bind("application"), fill_horizontal = 1, width_in_chars = 55 },
                f:push_button {
                    title = "Choose Antitone Application…",
                    action = function()
                        dofile(_PLUGIN.path .. "/Run.lua")("configure", function(path)
                            if path then properties.application = path end
                        end)
                    end,
                },
            },
        }
    end,
}
