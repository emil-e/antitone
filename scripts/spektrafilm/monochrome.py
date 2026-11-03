#!/usr/bin/env python3
"""Generate/check factored B&W negative and paper components.

Uses the pinned environment, shared Hanatos spectrum family and nonspatial
algorithms, not Spektrafilm colour-stock profiles. Application builds use saved components.
"""

import argparse
import bisect
from dataclasses import replace
import hashlib
import importlib
import importlib.util
import json
import math
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import zlib

if __package__:
    from . import build
else:
    spec = importlib.util.spec_from_file_location(
        "paper_generation_build", Path(__file__).with_name("build.py")
    )
    build = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(build)

ROOT = Path(__file__).resolve().parents[2]
ASSETS = ROOT / "assets/monochrome"
SOURCES = sorted((ROOT / "assets/papers").glob("*/source.json")) + sorted(
    (ROOT / "assets/negatives").glob("*/source.json")
)
FIXTURE = ROOT / "tests/fixtures/monochrome/conformance.json.z"
PRODUCERS = [
    Path(__file__).resolve(),
    Path(build.__file__).resolve(),
    build.LOCK,
    *SOURCES,
    ROOT / "native/spektrafilm/reference.cpp",
    ROOT / "native/rendering_transform.h",
    ROOT / "assets/ocio/authoring-reference.json",
]


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def reconstructed_density(parameters, log_h):
    low, high, slope, midpoint, shape = parameters
    z = -slope * (log_h - midpoint)
    return low + (high - low) * math.exp(-shape * math.log1p(math.exp(z)))


def source_density(source, log_h):
    reconstruction = source["reconstruction"]
    if reconstruction["family"] == "generalized-logistic-5":
        return reconstructed_density(reconstruction["parameters"][0], log_h)
    if reconstruction["family"] == "tabulated":
        samples = reconstruction["samples"]
        index = bisect.bisect_right([row[0] for row in samples], log_h)
        if index == 0:
            return samples[0][1]
        if index == len(samples):
            return samples[-1][1]
        a, b = samples[index - 1], samples[index]
        fraction = (log_h - a[0]) / (b[0] - a[0])
        return a[1] + (b[1] - a[1]) * fraction
    raise ValueError("unreviewed characteristic-curve reconstruction")


def sensitivity_response(sensitivity, wavelength):
    scale = sensitivity.get("scale")
    if scale not in ("linear-relative", "log10-reciprocal-exposure"):
        raise ValueError("unreviewed spectral-sensitivity scale")
    samples = sensitivity["samples"]
    if wavelength < samples[0][0] or wavelength > samples[-1][0]:
        return 0.0
    index = bisect.bisect_right([row[0] for row in samples], wavelength)
    if index == len(samples):
        value = samples[-1][1]
    else:
        a, b = samples[index - 1], samples[index]
        fraction = (wavelength - a[0]) / (b[0] - a[0])
        value = a[1] * (1 - fraction) + b[1] * fraction
    return value if scale == "linear-relative" else 10**value


def audit_source(source):
    samples = source["reconstruction"].get("samples")
    if samples is not None and (
        len(samples) < 2
        or any(not math.isfinite(v) for row in samples for v in row)
        or any(b[0] <= a[0] or b[1] < a[1] for a, b in zip(samples, samples[1:]))
    ):
        raise ValueError("invalid characteristic-curve samples")
    distances = []
    for sample in source["constraints"]:
        density = source_density(source, sample["log_exposure"])
        distances.append(
            min(max(lo - density, 0, density - hi) for lo, hi in sample["coverage_intervals"])
        )
    sensitivity = source.get("sensitivity")
    if sensitivity is not None:
        if sensitivity.get("scale") not in ("linear-relative", "log10-reciprocal-exposure"):
            raise ValueError("unreviewed spectral-sensitivity scale")
        samples = sensitivity["samples"]
        if (
            len(samples) < 2
            or any(not math.isfinite(v) for row in samples for v in row)
            or any(b[0] < a[0] for a, b in zip(samples, samples[1:]))
            or samples[0][0] >= samples[-1][0]
            or (sensitivity["scale"] == "linear-relative" and any(row[1] < 0 for row in samples))
        ):
            raise ValueError("invalid spectral-sensitivity samples")
    maximum = max(distances)
    if maximum > source["reading_uncertainty"]["density"]:
        raise ValueError("characteristic curve outside retained graph-reading allowance")
    return {"constraints": len(distances), "max_density_outside_ink_interval": maximum}


def check(candidate):
    provenance = json.loads((candidate / "provenance.json").read_text())
    if provenance["upstream_revision"] != build.REVISION:
        raise ValueError("paper algorithm pin differs")
    for path in PRODUCERS:
        if provenance["inputs"][str(path.relative_to(ROOT))] != sha(path):
            raise ValueError("paper producer changed: " + str(path))
    if provenance["dependencies"] != build.requirements():
        raise ValueError("paper dependencies differ from generation lock")
    for name in ["components.json.z", "spectral.f32.z", "index.json", "conformance.json.z"]:
        path = (
            FIXTURE
            if candidate.resolve() == ASSETS.resolve() and name == "conformance.json.z"
            else candidate / name
        )
        if provenance["outputs"][name] != sha(path):
            raise ValueError("paper output changed: " + name)
    catalogue = json.loads(zlib.decompress((candidate / "components.json.z").read_bytes()))
    index = json.loads((candidate / "index.json").read_text())
    for kind, root in [("paper", "papers"), ("negative", "negatives")]:
        sources = [json.loads(p.read_text()) for p in SOURCES if p.parent.parent.name == root]
        if [s["material"] for s in sources] != [p["material"] for p in catalogue[kind]]:
            raise ValueError("material identities differ from source")
        if [p["material"]["id"] for p in catalogue[kind]] != [p["id"] for p in index[kind]]:
            raise ValueError("catalogue index differs")
        for source in sources:
            audit_source(source)
    if (
        candidate.resolve() == ASSETS.resolve()
        and sha(FIXTURE) != provenance["outputs"]["conformance.json.z"]
    ):
        raise ValueError("maintained paper fixture differs")
    print("Paper provenance, source constraints and component bytes verified.")


def generate(upstream, output, scratch):
    dependencies = build.verify_environment()
    upstream = build.verify_source(upstream)
    output = output.resolve()
    if output.exists() or not output.is_relative_to((ROOT / ".cache").resolve()):
        raise ValueError("choose a fresh output directory under checkout .cache/")
    os.environ["NUMBA_CACHE_DIR"] = str(scratch / "numba")
    os.environ["MPLCONFIGDIR"] = str(scratch / "matplotlib")
    for key in [
        "NUMBA_NUM_THREADS",
        "OPENBLAS_NUM_THREADS",
        "OMP_NUM_THREADS",
        "VECLIB_MAXIMUM_THREADS",
    ]:
        os.environ[key] = "4"
    sys.dont_write_bytecode = True
    sys.path.insert(0, str(upstream / "src"))
    np = importlib.import_module("numpy")
    colour = importlib.import_module("colour")
    optimize = importlib.import_module("scipy.optimize")
    api = importlib.import_module("spektrafilm.runtime.api")
    develop = importlib.import_module("spektrafilm.model.develop")
    conversions = importlib.import_module("spektrafilm.utils.conversions")
    config = importlib.import_module("spektrafilm.config")
    illuminants = importlib.import_module("spektrafilm.model.illuminants")
    gamut = importlib.import_module("spektrafilm.utils.gamut_compression")

    reference = json.loads((ROOT / "assets/ocio/authoring-reference.json").read_text())[
        "reference"
    ]["parameters"]
    prefix = Path(os.environ.get("OCIO_PREFIX", ROOT / ".cache/deps/ocio-install")).resolve()
    helper = scratch / "reference"
    subprocess.run(
        [
            os.environ.get("CXX", "/usr/bin/clang++"),
            "-std=c++17",
            "-O3",
            "-Wall",
            "-Wextra",
            "-Werror",
            "-I" + str(prefix / "include"),
            str(ROOT / "native/spektrafilm/reference.cpp"),
            "-L" + str(prefix / "lib"),
            "-Wl,-rpath," + str(prefix / "lib"),
            "-lOpenColorIO",
            "-o",
            str(helper),
        ],
        check=True,
    )

    def ocio(values, direction):
        values = np.asarray(values, dtype="<f4").reshape(-1, 3)
        input_path, output_path = scratch / "input.f32", scratch / "output.f32"
        values.tofile(input_path)
        subprocess.run(
            [
                str(helper),
                direction,
                str(input_path),
                str(output_path),
                *[format(v, ".17g") for v in reference],
            ],
            check=True,
        )
        result = np.fromfile(output_path, dtype="<f4").reshape(-1, 3).astype(float)
        if result.shape != values.shape or not np.isfinite(result).all():
            raise ValueError("invalid authoring-reference result")
        return result

    def cct(values):
        values = np.asarray(values)
        return np.where(
            values <= 0.0078125,
            10.5402377416545 * values + 0.0729055341958355,
            (np.log2(np.maximum(values, 1e-300)) + 9.72) / 17.52,
        )

    spectral = importlib.import_module("spektrafilm.utils.spectral_upsampling")
    sources = [(path, json.loads(path.read_text())) for path in SOURCES]
    audits = {s["material"]["id"]: audit_source(s) for _, s in sources}

    def curve_component(source):
        h_min, h_max = source["material"]["log_exposure_range"]
        axis = np.linspace(h_min, h_max, 2049)
        densities = np.array([source_density(source, h) for h in axis])
        if not np.isfinite(densities).all() or np.any(np.diff(densities) < 0):
            raise ValueError("generated characteristic table is not finite and monotone")
        # Repetition adapts a scalar response to upstream's RGB-shaped API.
        curves = np.repeat(densities[:, None], 3, axis=1)

        def density(log_h):
            log_h = np.asarray(log_h).reshape(-1)
            values = np.repeat(log_h[None, :, None], 3, axis=2)
            return develop.develop_simple(values, axis, curves)[0, :, 0]

        dense_h = np.linspace(h_min, h_max, 32769)
        expected = np.array([source_density(source, h) for h in dense_h])
        error = float(np.max(np.abs(density(dense_h) - expected)))
        if error > 1e-5:
            raise ValueError(f"characteristic table approximation exceeds budget: {error}")
        return {"min": h_min, "max": h_max, "density": densities.tolist()}, density, error

    # Only global finishing defaults are read here. None of either colour
    # material's sensitivities, curves, densities, DIR or reference corrections
    # enters the one-absorber calculation below.
    defaults = api.init_params("kodak_portra_400", "kodak_portra_endura")
    finishing = defaults.io.output_gamut_compress
    observer = config.STANDARD_OBSERVER_CMFS[:]
    light = np.asarray(illuminants.standard_illuminant("D50"))
    normalization = np.sum(light * observer[:, 1])
    white_xyz = light @ observer / normalization
    ref_xyz = colour.normalised_primary_matrix(
        np.array(reference[1:7]).reshape(3, 2), np.array(reference[7:9])
    )
    srgb = colour.RGB_COLOURSPACES["sRGB"]
    srgb_xyz = colour.normalised_primary_matrix(srgb.primaries, srgb.whitepoint)
    to_reference = (
        np.linalg.inv(ref_xyz)
        @ colour.adaptation.matrix_chromatic_adaptation_VonKries(
            colour.xy_to_XYZ(srgb.whitepoint),
            colour.xy_to_XYZ(reference[7:9]),
            transform="Bradford",
        )
        @ srgb_xyz
    )

    def reflection_xyz(density_values):
        values = np.asarray(density_values).reshape(1, -1, 1)
        spectral_density = develop.compute_density_spectral(np.ones((len(light), 1)), values)
        reflected = conversions.density_to_light(spectral_density, light)
        xyz = reflected @ observer / normalization
        # A wavelength-independent reflection must preserve R as normalized Y.
        # This independent identity catches an extra base or replicated density.
        if np.max(np.abs(xyz[0, :, 1] - 10 ** (-values[0, :, 0]))) > 1e-12:
            raise ValueError("neutral reflection differs from one total density")
        return xyz

    def readout(density_values):
        xyz = reflection_xyz(density_values)
        # ScanningStage retains this finite-light epsilon before RGB conversion.
        xyz = 10 ** np.log10(np.maximum(xyz, 0) + 1e-10)
        rgb = colour.XYZ_to_RGB(
            xyz,
            colourspace="sRGB",
            apply_cctf_encoding=False,
            illuminant=colour.XYZ_to_xy(white_xyz),
        )
        rgb = gamut.compress_rgb(rgb, finishing, output_color_space="sRGB")
        return rgb.reshape(-1, 3) @ to_reference.T

    gray = 0.18
    target = float(ocio([[gray] * 3], "forward")[0] @ ref_xyz[1])
    catalogue = {"paper": [], "negative": []}
    index = {"paper": [], "negative": []}
    paper_models, film_models, raw_parts = [], [], []
    curve_errors = {}
    xy = spectral._illuminant_to_xy("D55")
    input_matrix = colour.RGB_to_XYZ(
        np.eye(3),
        "ACEScg",
        illuminant=xy,
        chromatic_adaptation_transform="CAT16",
        apply_cctf_decoding=False,
    ).T
    for path, source in sources:
        material = source["material"]
        kind = "paper" if path.parent.parent.name == "papers" else "negative"
        exposure, density, error = curve_component(source)
        curve_errors[material["id"]] = error
        label = {
            "id": material["id"],
            "name": material["name"],
            "citation": source["publication"] + ", " + source["url"],
        }
        index[kind].append(label)
        component = {"material": material, "exposure": exposure}
        if kind == "paper":
            gray_h = optimize.brentq(
                lambda h: float(readout(density([h]))[0] @ ref_xyz[1]) - target,
                exposure["min"],
                exposure["max"],
                xtol=1e-13,
            )
            component["positive"] = {
                "gray_log_h": gray_h,
                "gray_ap1": gray,
                "exposure_compression": 0.6,
            }
            paper_models.append((component, density))
        else:
            wavelengths = np.arange(380, 781, 5)
            sensitivity = np.array(
                [
                    sensitivity_response(source["sensitivity"], wavelength)
                    for wavelength in wavelengths
                ]
            )
            # Scalar silver response, repeated only for the upstream API shape.
            sensitivity = np.repeat(sensitivity[:, None], 3, axis=1)
            raw_lut = spectral.compute_hanatos2025_tc_lut(
                sensitivity,
                replace(spectral.HANATOS2025_NO_ADAPTATION, reference_illuminant="D55"),
                gamut_compress=defaults.io.input_gamut_compress,
            )

            def raw_response(rgb, lut=raw_lut, sensitivity=sensitivity):
                return spectral.rgb_to_raw_hanatos2025(
                    np.asarray(rgb).reshape(1, -1, 3),
                    sensitivity,
                    color_space="ACEScg",
                    apply_cctf_decoding=False,
                    reference_illuminant="D55",
                    tc_lut=lut,
                )[0, :, 0]

            raw_gray = float(raw_response([[gray] * 3])[0])
            h_axis = np.linspace(exposure["min"], exposure["max"], len(exposure["density"]))
            # One relative exposure convention for every stock: gray is four
            # stops above the source curve's Dmin+0.1 point, not an ISO claim.
            gray_h = float(
                np.interp(exposure["density"][0] + 0.1, exposure["density"], h_axis)
                + 4 * np.log10(2)
            )
            raw = (raw_lut[:, :, 0] / raw_gray).astype("<f4")
            component.update(
                {
                    "matrix": input_matrix.tolist(),
                    "gray_log_h": gray_h,
                    "raw_size": len(raw),
                    "raw_offset": sum(v.size for v in raw_parts),
                }
            )
            raw_parts.append(raw)
            film_models.append((component, density, raw_response, raw_gray))
        catalogue[kind].append(component)

    d_min = min(p["exposure"]["density"][0] for p in catalogue["paper"])
    d_max = max(p["exposure"]["density"][-1] for p in catalogue["paper"])
    density_axis = np.linspace(d_min, d_max, 2049)
    encoded = cct(ocio(readout(density_axis), "inverse"))
    catalogue["readout"] = {"min": d_min, "max": d_max, "encoded": encoded.tolist()}
    dense_density = np.linspace(d_min, d_max, 16385)
    exact_encoded = cct(ocio(readout(dense_density), "inverse"))
    interpolated = np.column_stack(
        [np.interp(dense_density, density_axis, encoded[:, c]) for c in range(3)]
    )
    readout_error = float(np.max(np.abs(interpolated - exact_encoded)))
    if readout_error > 2e-5:
        raise ValueError(f"neutral readout approximation exceeds budget: {readout_error}")
    ap1 = colour.RGB_COLOURSPACES["ACEScg"]
    ap1_y = colour.normalised_primary_matrix(ap1.primaries, ap1.whitepoint)[1]
    inputs = np.array(
        [
            [0, 0, 0],
            [0.001] * 3,
            [0.01] * 3,
            [0.18] * 3,
            [1] * 3,
            [16] * 3,
            [-1] * 3,
            [1, 0, 0],
            [0, 1, 0],
            [0, 0, 1],
            [-0.1, 0.3, 0.01],
            [4, -0.1, 0.01],
            [0.7, 0.08, -0.01],
            [0.02, 0.5, 0.1],
            [0.04, 0.01, 2],
        ]
    )
    # Signed/HDR channel mixtures exercise luminance before nonlinear exposure,
    # rather than merely agreeing along the neutral axis.
    encoded_inputs = np.random.default_rng(761923).uniform(-0.5, 1.0, (1024, 3))
    random_inputs = np.where(
        encoded_inputs <= 0.155251141552511,
        (encoded_inputs - 0.0729055341958355) / 10.5402377416545,
        2 ** (17.52 * encoded_inputs - 9.72),
    )
    equal_y = np.array([gray + 0.2, gray - 0.2 * ap1_y[0] / ap1_y[1], gray])
    inputs = np.vstack(
        [inputs, equal_y, random_inputs, np.linspace(-0.02, 4, 257)[:, None] * np.ones(3)]
    )
    fixture = {
        "inputs": inputs.tolist(),
        "cases": [],
        "reference": "Pinned upstream unwindowed Hanatos projection, scalar develop_simple, neutral transmission/reflection, D50/CIE1931, scan conversion/finishing and inverse authoring DRT. Numerical reference, not physical measurements.",
    }
    for paper, paper_density in paper_models:
        for film_model in [None, *film_models]:
            if film_model is None:
                film_id, raw, film_h, film_d = None, None, None, None
                h = paper["positive"]["gray_log_h"] - 17.52 * (cct(inputs @ ap1_y) - cct(gray)) * (
                    0.6 * np.log10(2)
                )
            else:
                film, film_density, response, raw_gray = film_model
                film_id = film["material"]["id"]
                raw = response(inputs) / raw_gray
                film_h = film["gray_log_h"] + np.log10(np.maximum(raw, 0) + 1e-10)
                film_d = film_density(film_h)
                gray_d = float(film_density([film["gray_log_h"] + np.log10(1 + 1e-10)])[0])
                h = paper["positive"]["gray_log_h"] + gray_d - film_d
            d = paper_density(h)
            rgb = readout(d)
            fixture["cases"].append(
                {
                    "negative": film_id,
                    "paper": paper["material"]["id"],
                    "raw": None if raw is None else raw.tolist(),
                    "film_log_h": None if film_h is None else film_h.tolist(),
                    "film_density": None if film_d is None else film_d.tolist(),
                    "log_h": h.tolist(),
                    "density": d.tolist(),
                    "reflection_y": reflection_xyz(d)[0, :, 1].tolist(),
                    "encoded": cct(ocio(rgb, "inverse")).tolist(),
                    "reference_rgb": rgb.tolist(),
                }
            )
    output.mkdir()
    payloads = {
        "components.json.z": zlib.compress(
            json.dumps(catalogue, separators=(",", ":")).encode(), 9
        ),
        "spectral.f32.z": zlib.compress(
            np.concatenate([p.reshape(-1) for p in raw_parts]).tobytes(), 9
        ),
        "index.json": (json.dumps(index, indent=2) + "\n").encode(),
        "conformance.json.z": zlib.compress(json.dumps(fixture, separators=(",", ":")).encode(), 9),
    }
    for name, content in payloads.items():
        (output / name).write_bytes(content)
    provenance = {
        "upstream_revision": build.REVISION,
        "inputs": {str(p.relative_to(ROOT)): sha(p) for p in PRODUCERS},
        "dependencies": dependencies,
        "conditions": {s["material"]["id"]: s["conditions"] for _, s in sources},
        "model": "Scalar total film density with neutral transmission; fixed-grade paper total reflection density, no extra base. Paper-only uses luminance and0.6-stop exposure convention. No colour-stock data, DIR, invented paper sensitivities or tint.",
        "spectral": "Shared Hanatos family, no stock-adaptation window/surface, CAT16/D55 input and configured input-gamut compression; published sensitivity support only. Estimated scene spectrum, not recovered radiance.",
        "finishing": str(finishing),
        "viewing": "D50, CIE1931, CAT02 to linear sRGB, global upstream finishing, shared inverse authoring DRT",
        "calibration": {
            "reference_gray_y": target,
            "film_gray_placement_stops_above_Dmin_plus_0_1": 4,
            "print_time": "Paper gray logH plus developed film density at AP1gray0.18; only scalar printing exposure changes.",
        },
        "approximation": {
            "max_density_errors": curve_errors,
            "max_encoded_readout_error": readout_error,
        },
        "source_audit": audits,
        "outputs": {name: sha(output / name) for name in payloads},
    }
    (output / "provenance.json").write_text(json.dumps(provenance, indent=2) + "\n")
    check(output)
    print(json.dumps(provenance["approximation"], indent=2))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["generate", "check"])
    parser.add_argument("--upstream", type=Path, default=build.UPSTREAM)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    if args.command == "check":
        check(args.output or ASSETS)
    elif args.output is None:
        parser.error("generate requires --output")
    else:
        cache = ROOT / ".cache"
        cache.mkdir(exist_ok=True)
        with tempfile.TemporaryDirectory(prefix="monochrome-export-", dir=cache) as temporary:
            generate(args.upstream, args.output, Path(temporary))


if __name__ == "__main__":
    main()
