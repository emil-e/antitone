#pragma once
#include <cmath>
#include <cstddef>
#include <cstdio>
#include <string>
#include <utility>
#include <variant>
#include <vector>

struct Json {
    using Array = std::vector<Json>;
    using Object = std::vector<std::pair<std::string, Json>>;

    std::variant<std::nullptr_t, bool, double, Array, Object> value;

    Json(std::nullptr_t) : value(nullptr) {}
    Json(bool boolean) : value(boolean) {}
    Json(double number) : value(number) {}
    Json(Array array) : value(std::move(array)) {}
    Json(Object object) : value(std::move(object)) {}

    static Json null() { return Json(nullptr); }
    static Json boolean(bool value) { return Json(value); }
    static Json number(double value) { return Json(value); }
    static Json array(Array values) { return Json(std::move(values)); }
    static Json object(Object values) { return Json(std::move(values)); }

    void write(std::string &out) const {
        if (std::holds_alternative<std::nullptr_t>(value)) {
            out += "null";
        } else if (auto *boolean = std::get_if<bool>(&value)) {
            out += *boolean ? "true" : "false";
        } else if (auto *number = std::get_if<double>(&value)) {
            write_number(out, *number);
        } else if (auto *array = std::get_if<Array>(&value)) {
            out += '[';
            for (size_t i = 0; i < array->size(); ++i) {
                if (i)
                    out += ',';
                (*array)[i].write(out);
            }
            out += ']';
        } else if (auto *object = std::get_if<Object>(&value)) {
            out += '{';
            for (size_t i = 0; i < object->size(); ++i) {
                if (i)
                    out += ',';
                write_string(out, (*object)[i].first);
                out += ':';
                (*object)[i].second.write(out);
            }
            out += '}';
        }
    }

private:
    static void write_number(std::string &out, double number) {
        if (!std::isfinite(number)) {
            out += "null";
            return;
        }
        char buffer[64];
        std::snprintf(buffer, sizeof buffer, "%.17g", number);
        out += buffer;
    }

    static void write_string(std::string &out, const std::string &text) {
        out += '"';
        for (char c : text) {
            if (c == '"' || c == '\\') {
                out += '\\';
                out += c;
            } else if (static_cast<unsigned char>(c) < 0x20) {
                char escaped[7];
                std::snprintf(escaped, sizeof escaped, "\\u%04x", static_cast<unsigned char>(c));
                out += escaped;
            } else {
                out += c;
            }
        }
        out += '"';
    }
};
