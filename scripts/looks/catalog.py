"""Versioned independent look definitions. No native build or private data access."""
from dataclasses import dataclass
from pathlib import Path
import hashlib
import json
import math
import re

ROOT = Path(__file__).resolve().parents[2]
ASSETS = ROOT / "assets/looks"
ID = re.compile(r"[a-z0-9]+(?:-[a-z0-9]+)*\Z")
SHA = re.compile(r"[0-9a-f]{64}\Z")


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def read_json(path):
    def pairs(items):
        result = {}
        for key, value in items:
            if key in result:
                raise ValueError(f"duplicate JSON key: {key}")
            result[key] = value
        return result

    def constant(value):
        raise ValueError(f"nonfinite JSON constant: {value}")

    def real(text):
        value = float(text)
        require(math.isfinite(value), "nonfinite JSON number")
        return value

    def integer(text):
        value = int(text)
        try:
            finite = math.isfinite(value)
        except OverflowError:
            finite = False
        require(finite, "JSON integer outside finite numeric range")
        return value

    return json.loads(Path(path).read_text(), object_pairs_hook=pairs, parse_constant=constant,
                      parse_float=real, parse_int=integer)


def require(condition, message):
    if not condition:
        raise ValueError(message)


def keys(value, required, optional=()):
    require(isinstance(value, dict), "expected an object")
    require(set(required) <= value.keys() <= set(required) | set(optional),
            f"unexpected/missing fields: {sorted(value)}")


def numbers(value, shape, nonnegative=False):
    if not shape:
        require(type(value) in (int, float) and math.isfinite(value), "expected finite number")
        require(not nonnegative or value >= 0, "negative density/coefficient")
        return
    require(isinstance(value, list) and len(value) == shape[0], f"expected array shape {shape}")
    for item in value:
        numbers(item, shape[1:], nonnegative)


def revision_one(value):
    return type(value) is int and value == 1


def parameters(rows, family, channels):
    width = {"generalized-logistic-5": 5, "monotone-mixture-8": 8}.get(family)
    require(width is not None, "unsupported source reconstruction")
    numbers(rows, (channels, width))
    for p in rows:
        require(0 <= p[0] < p[1] and p[2] > 0 and p[4] > 0, "nonmonotone source parameters")
        if width == 8:
            require(0 <= p[5] <= 1 and p[6] > 0, "invalid monotone mixture")


def common(assets=ASSETS):
    authoring = read_json(assets / "common/authoring.json")
    keys(authoring, ("schema_version", "id", "ocio_version", "exposure_compression", "pivot_ap1", "anchor", "input_output", "input_domain", "interpolation", "supported_cube_sizes", "reference", "viewing", "source_density_allowance", "reference_roundtrip_limit", "native_ap1_range", "gray_ap1_limit", "raw_gray_limit", "file_local_tetra_limit"))
    require(revision_one(authoring["schema_version"]) and authoring["id"] == "fixed-print-reference-v1", "unsupported authoring version")
    # This revision deliberately has one fixed, reviewed contract, not a set
    # of knobs that can silently change the meaning of an existing look ID.
    require(authoring["ocio_version"] == "2.6.0" and authoring["exposure_compression"] == .6
            and authoring["pivot_ap1"] == .18 and authoring["interpolation"] == "tetrahedral"
            and authoring["input_domain"] == [-.5, 1.0386752042]
            and authoring["supported_cube_sizes"] == [65, 97], "unsupported authoring contract")
    require(authoring["anchor"] == "gray-only" and authoring["input_output"] == "ACEScct/AP1/D60", "unsupported exposure/space convention")
    require(authoring["reference"] == {"id": "p3rec50", "parameters": [100., .694, .306, .2175, .7435, .1405, .053, .3127, .3290], "boundary_start_radius": .9}, "unsupported reference")
    expected_gates = {"source_density_allowance": .025, "reference_roundtrip_limit": 2e-4,
                      "native_ap1_range": [0., 1024.], "gray_ap1_limit": 2e-5,
                      "raw_gray_limit": 1e-9, "file_local_tetra_limit": 2e-4}
    require(all(authoring.get(k) == v for k, v in expected_gates.items()), "changed admission gates")
    spectral = read_json(assets / "common/spectral.json")
    keys(spectral, ("schema_version", "wavelengths", "observer", "illumination", "observer_source", "illumination_model"))
    require(revision_one(spectral["schema_version"]) and spectral["wavelengths"] == list(range(400, 701, 5)), "unsupported spectral grid")
    numbers(spectral["observer"], (61, 3))
    numbers(spectral["illumination"], (61,), True)
    require(all(v > 0 for v in spectral["illumination"]), "nonpositive illuminant")
    status = read_json(assets / "common/status-a.json")
    for channel in ("red", "green", "blue"):
        product = status[channel]
        require(len(product["points"]) >= 2, "empty meter product")
        for point in product["points"]:
            numbers(point, (2,))
        require(all(a[0] < b[0] for a, b in zip(product["points"], product["points"][1:])), "unsorted meter wavelengths")
        numbers(product["short_wavelength_log_slope_per_nm"], ())
        numbers(product["long_wavelength_log_slope_per_nm"], ())
    return authoring, spectral, status


@dataclass(frozen=True)
class Definition:
    path: Path
    look: dict
    model: dict
    source: dict

    @property
    def id(self):
        return self.look["id"]

    @property
    def monochrome(self):
        return self.model["family"] == "neutral-density-analytic"


def load(identifier, assets=ASSETS):
    require(isinstance(identifier, str) and ID.fullmatch(identifier), "invalid look ID")
    path = assets / identifier
    look = read_json(path / "look.json")
    keys(look, ("schema_version", "id", "version", "name", "status", "material", "authoring", "model_sha256", "source_sha256", "reviewed_bakes", "notes", "acceptance_scope", "release_clearance"))
    require(revision_one(look["schema_version"]) and revision_one(look["version"]) and look["id"] == identifier, "unsupported/mismatched look identity")
    require(look["status"] in ("selected", "accepted", "candidate", "shelved"), "invalid selection status")
    require(look["authoring"] == "fixed-print-reference-v1" and look["release_clearance"] == "unresolved", "unsupported authoring/release contract")
    require(isinstance(look["name"], str) and look["name"] and all(ord(c) >= 32 for c in look["name"]), "invalid look label")
    for name in ("model", "source"):
        require(isinstance(look[name + "_sha256"], str) and SHA.fullmatch(look[name + "_sha256"]), "invalid data digest")
        require(digest(path / (name + ".json")) == look[name + "_sha256"], f"{identifier}: changed frozen {name} data")
    require(set(look["reviewed_bakes"]) == {"65", "97"}, "missing reviewed lattice identities")
    for bake in look["reviewed_bakes"].values():
        keys(bake, ("numeric_payload_sha256", "reviewed_file_sha256"))
        require(all(isinstance(v, str) and SHA.fullmatch(v) for v in bake.values()), "invalid bake digest")
    model = read_json(path / "model.json")
    require(revision_one(model["schema_version"]), "unsupported model schema")
    family = model["family"]
    if family == "spectral-density-table":
        keys(model, ("schema_version", "family", "log_exposure", "dye_density", "effective_coefficients", "separate_base"))
        numbers(model["log_exposure"], (256,))
        h = model["log_exposure"]
        require(h[0] < h[-1] and all(abs(v - (h[0] + (h[-1] - h[0]) * i / 255)) < 1e-12 for i, v in enumerate(h)), "nonuniform exposure grid")
        numbers(model["dye_density"], (61, 3), True)
        numbers(model["effective_coefficients"], (256, 3), True)
        require(model["separate_base"] is None, "revision 1 includes Dmin in effective coefficients; no separate base")
        require(look["material"] in ("reflection", "transmission"), "spectral material kind mismatch")
    elif family == "neutral-density-analytic":
        keys(model, ("schema_version", "family", "log_exposure_range", "density_parameters"))
        numbers(model["log_exposure_range"], (2,))
        require(model["log_exposure_range"][0] < model["log_exposure_range"][1], "empty scalar exposure range")
        parameters([model["density_parameters"]], "generalized-logistic-5", 1)
        require(look["material"] == "neutral-reflection", "scalar material kind mismatch")
    else:
        raise ValueError("unsupported material evaluator")
    source = read_json(path / "source.json")
    keys(source, ("schema_version", "publication", "url", "characteristic_page", "conditions", "reading_uncertainty", "reconstruction", "constraints", "constraints_scope"), ("dye_page", "dye_readings"))
    require(revision_one(source["schema_version"]) and isinstance(source["url"], str) and source["url"].startswith("https://"), "invalid source identity")
    require(isinstance(source["publication"], str) and bool(source["publication"]) and type(source["characteristic_page"]) is int and source["characteristic_page"] > 0, "missing original publication/page")
    require(isinstance(source["conditions"], dict), "missing processing conditions")
    if family == "spectral-density-table":
        require(set(source.get("dye_readings", {})) == {"cyan", "magenta", "yellow"} and type(source.get("dye_page")) is int and source["dye_page"] > 0, "missing original dye observations/page")
        for rows in source["dye_readings"].values():
            require(isinstance(rows, list) and len(rows) == 31, "unexpected dye observation count")
            for row in rows:
                keys(row, ("nm", "density", "censored_below_graph_resolution"))
                numbers(row["nm"], ())
                numbers(row["density"], (), True)
                require(type(row["censored_below_graph_resolution"]) is bool, "invalid censor flag")
                require(not row["censored_below_graph_resolution"] or row["density"] == 0, "censored tail must retain the declared zero convention")
    uncertainty_fields = ("density", "log_exposure")
    if family == "spectral-density-table":
        uncertainty_fields += ("relative_dye_density", "wavelength_nm")
    keys(source["reading_uncertainty"], uncertainty_fields)
    require(source["reading_uncertainty"]["density"] == .025, "changed source density allowance")
    require(all(type(v) in (int, float) and math.isfinite(v) and v > 0 for v in source["reading_uncertainty"].values()), "invalid source uncertainty")
    rec = source["reconstruction"]
    keys(rec, ("family", "parameters"))
    parameters(rec["parameters"], rec["family"], 1 if family == "neutral-density-analytic" else 3)
    require(bool(source["constraints"]), "missing numerical source evidence")
    for row in source["constraints"]:
        keys(row, ("log_exposure", "channel_intervals", "resolved", "coverage_intervals", "occluded_intervals"))
        numbers(row["log_exposure"], ())
        channels = 1 if family == "neutral-density-analytic" else 3
        numbers(row["channel_intervals"], (channels, 2))
        require(len(row["resolved"]) == channels and all(type(v) is bool for v in row["resolved"]), "invalid source resolution flags")
        for intervals in (row["channel_intervals"], row["coverage_intervals"], row["occluded_intervals"]):
            for span in intervals:
                numbers(span, (2,))
                require(span[0] <= span[1], "reversed source interval")
    return Definition(path, look, model, source)


def identifiers(assets=ASSETS):
    return sorted(p.name for p in assets.iterdir() if p.is_dir() and p.name != "common")


def payload_hash(path):
    lines = Path(path).read_bytes().splitlines(keepends=True)
    starts = [i for i, line in enumerate(lines) if line.startswith(b"LUT_3D_SIZE ")]
    require(len(starts) == 1, "expected one 3D LUT payload")
    return hashlib.sha256(b"".join(lines[starts[0]:])).hexdigest()
