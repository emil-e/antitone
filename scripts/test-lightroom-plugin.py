#!/usr/bin/env python3
"""Run the Lightroom plug-in's Lua SDK mocks without launching Lightroom."""

import argparse
import ctypes
import ctypes.util
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CLASSIC_LUA = Path(
    "/Applications/Adobe Lightroom Classic/Adobe Lightroom Classic.app"
    "/Contents/Frameworks/AgKernel.framework/Versions/A/AgKernel"
)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--lua-library",
        help="Lua 5.1 shared library; defaults to installed Lightroom or a system Lua 5.1 library",
    )
    parser.add_argument(
        "--plugin",
        type=Path,
        default=ROOT / "integrations/lightroom/Antitone.lrplugin",
        help="Bundle to test; can be a freshly extracted distribution",
    )
    args = parser.parse_args()
    library_path = args.lua_library or (
        str(CLASSIC_LUA) if CLASSIC_LUA.is_file() else ctypes.util.find_library("lua5.1")
    )
    if not library_path:
        parser.error("Lua 5.1 library not found; supply --lua-library")

    library = ctypes.CDLL(library_path)
    state_type = ctypes.c_void_p
    library.luaL_newstate.restype = state_type
    library.luaL_openlibs.argtypes = [state_type]
    library.luaL_loadfile.argtypes = [state_type, ctypes.c_char_p]
    library.lua_pcall.argtypes = [state_type, ctypes.c_int, ctypes.c_int, ctypes.c_int]
    library.lua_tolstring.argtypes = [state_type, ctypes.c_int, ctypes.POINTER(ctypes.c_size_t)]
    library.lua_tolstring.restype = ctypes.c_void_p
    library.lua_pushlstring.argtypes = [state_type, ctypes.c_char_p, ctypes.c_size_t]
    library.lua_pushcclosure.argtypes = [state_type, ctypes.c_void_p, ctypes.c_int]
    library.lua_pushboolean.argtypes = [state_type, ctypes.c_int]
    library.lua_setfield.argtypes = [state_type, ctypes.c_int, ctypes.c_char_p]
    library.lua_close.argtypes = [state_type]
    callback_type = ctypes.CFUNCTYPE(ctypes.c_int, state_type)
    globals_index = -10002

    def string(state, index):
        size = ctypes.c_size_t()
        pointer = library.lua_tolstring(state, index, ctypes.byref(size))
        return ctypes.string_at(pointer, size.value)

    reference = json.loads((ROOT / "tests/lightroom/request.json").read_text(encoding="utf-8"))
    json_errors = []

    @callback_type
    def validate_request(state):
        try:
            actual = json.loads(string(state, 1).decode())
            assert actual == reference, "request differs from shared JSON fixture"
            valid = True
        except (UnicodeError, ValueError, AssertionError) as error:
            json_errors.append(str(error))
            valid = False
        library.lua_pushboolean(state, valid)
        return 1

    state = library.luaL_newstate()
    if not state:
        raise RuntimeError("Lua state allocation failed")
    try:
        library.luaL_openlibs(state)
        library.lua_pushcclosure(state, validate_request, 0)
        library.lua_setfield(state, globals_index, b"validateRequestJson")
        for name, fixture in [(b"RECEIPT_FIXTURE", "result.json"), (b"SAVE_FIXTURE", "saved.json")]:
            receipt = (ROOT / "tests/lightroom" / fixture).read_bytes()
            library.lua_pushlstring(state, receipt, len(receipt))
            library.lua_setfield(state, globals_index, name)
        test_root = (ROOT / "tests/lightroom").as_posix().encode()
        library.lua_pushlstring(state, test_root, len(test_root))
        library.lua_setfield(state, globals_index, b"TEST_ROOT")
        plugin = args.plugin.resolve().as_posix().encode()
        library.lua_pushlstring(state, plugin, len(plugin))
        library.lua_setfield(state, globals_index, b"PLUGIN_PATH")
        script = (ROOT / "tests/lightroom/plugin.lua").as_posix().encode()
        status = library.luaL_loadfile(state, script)
        if not status:
            status = library.lua_pcall(state, 0, 0, 0)
        if status:
            raise RuntimeError("\n".join([string(state, -1).decode(), *json_errors]))
    finally:
        library.lua_close(state)


if __name__ == "__main__":
    main()
