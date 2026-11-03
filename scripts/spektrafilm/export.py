#!/usr/bin/env python3
"""Export spektrafilm's nonspatial functions/data for the native application.

Run in the pinned upstream environment. Upstream is a generation dependency only.
No photograph inputs, fitted colour transforms or runtime Python are involved.
"""

from pathlib import Path
import argparse
import hashlib
import importlib
import json
import os
import subprocess
import sys
import tempfile
import zlib

from build import LOCK, REVISION, verify_environment

ROOT = Path(__file__).resolve().parents[2]


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--upstream", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--scanner-size", type=int, default=33, choices=[17, 33, 65])
    args = parser.parse_args()
    cache = ROOT / ".cache"
    cache.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="spektrafilm-export-", dir=cache) as temporary:
        export(args, Path(temporary))


def export(args, scratch):
    dependencies = verify_environment()
    upstream = args.upstream.resolve()
    output = args.output.resolve()
    actual = subprocess.check_output(
        ["git", "-C", str(upstream), "rev-parse", "HEAD"], text=True
    ).strip()
    if actual != REVISION:
        raise ValueError("review a new upstream pin before regenerating")
    status = subprocess.check_output(
        ["git", "-C", str(upstream), "status", "--porcelain"], text=True
    ).strip()
    if status:
        raise ValueError("use an unchanged upstream checkout")
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
    Pipeline = importlib.import_module("spektrafilm.runtime.pipeline").SimulationPipeline
    spectral = importlib.import_module("spektrafilm.utils.spectral_upsampling")
    gamut = importlib.import_module("spektrafilm.utils.gamut_compression")
    couplers = importlib.import_module("spektrafilm.model.couplers")
    standard_illuminant = importlib.import_module(
        "spektrafilm.model.illuminants"
    ).standard_illuminant
    output.mkdir(parents=True)
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
        a = np.asarray(values, dtype="<f4").reshape(-1, 3)
        source = scratch / "input.f32"
        dest = scratch / "output.f32"
        a.tofile(source)
        subprocess.run(
            [
                str(helper),
                direction,
                str(source),
                str(dest),
                *[format(v, ".17g") for v in reference],
            ],
            check=True,
        )
        out = np.fromfile(dest, dtype="<f4").reshape(-1, 3)
        assert np.isfinite(out).all() and out.shape == a.shape
        return out.astype(float)

    def cct(v):
        return np.where(
            v <= 0.0078125,
            10.5402377416545 * v + 0.0729055341958355,
            (np.log2(np.maximum(v, 1e-300)) + 9.72) / 17.52,
        )

    refprim = np.array(reference[1:7]).reshape(3, 2)
    white = np.array(reference[7:9])
    refXYZ = colour.normalised_primary_matrix(refprim, white)
    srgb = colour.RGB_COLOURSPACES["sRGB"]
    srgb_xyz = colour.normalised_primary_matrix(srgb.primaries, srgb.whitepoint)
    to_ref = (
        np.linalg.inv(refXYZ)
        @ colour.adaptation.matrix_chromatic_adaptation_VonKries(
            colour.xy_to_XYZ(srgb.whitepoint), colour.xy_to_XYZ(white), transform="Bradford"
        )
        @ srgb_xyz
    )

    def reference_rgb(rgb):
        return np.asarray(rgb) @ to_ref.T

    gray_target = ocio([[0.18, 0.18, 0.18]], "forward")[0] @ np.linalg.inv(to_ref).T

    def params(film, paper):
        p = api.init_params(film, paper)
        p.io.input_color_space = "ACEScg"
        p.io.input_cctf_decoding = False
        p.io.output_color_space = "sRGB"
        p.io.output_cctf_encoding = False
        p.debug.lut_mode = True
        return api.digest_params(p)

    paths = sorted((upstream / "src/spektrafilm/data/profiles").glob("*.json"))
    infos = {p.stem: json.loads(p.read_text())["info"] for p in paths}
    negative_ids = [
        k for k, v in infos.items() if v["stage"] == "filming" and v["type"] == "negative"
    ]
    paper_ids = [k for k, v in infos.items() if v["stage"] == "printing"]
    spectra = spectral.HANATOS2025_SPECTRA_LUT.astype("<f2")
    assert spectra.shape[0] == spectra.shape[1] and spectra.shape[2] == 81
    n = len(spectra)
    (output / "irradiance_xy_tc.f16.z").write_bytes(zlib.compress(spectra.tobytes(), 9))
    values = np.linspace(0, 1, n)
    tc_cells = np.stack(np.meshgrid(values, values, indexing="ij"), axis=-1)
    catalog = {
        "schema": 1,
        "revision": REVISION,
        "spectra_size": n,
        "wavelengths": 81,
        "scanner_size": args.scanner_size,
        "negative": [],
        "paper": [],
        "pairs": [],
        "remap": {},
        "authoring_reference": reference,
        "print_gray_Y": 0.1,
        "paper_mapping": {"exposure_compression": 0.6, "gray_AP1": 0.18},
        "source_profile_sha256": {p.name: sha(p) for p in paths},
    }
    for name in negative_ids:
        p = params(name, "kodak_portra_endura")
        d = p.film.data
        curve = d.density_curves - np.nanmin(d.density_curves, axis=0)
        M = (
            couplers.compute_dir_couplers_matrix(p.film_render.dir_couplers)
            * p.film_render.dir_couplers.amount
            if p.film_render.dir_couplers.active
            else np.zeros((3, 3))
        )
        pre = (
            couplers.compute_density_curves_before_dir_couplers(curve, d.log_exposure, M)
            if p.film_render.dir_couplers.active
            else curve
        )
        sensitivity = np.nan_to_num(10**d.log_sensitivity)
        window = spectral.eval_spectral_bandpass_window(d.hanatos2025_adaptation_window_params)
        light = standard_illuminant(p.film.info.reference_illuminant)
        normalization = np.sum(sensitivity * light[:, None] * window, axis=0) / np.sum(
            sensitivity * light[:, None], axis=0
        )
        effective = sensitivity * window / normalization
        xy = spectral._illuminant_to_xy(p.film.info.reference_illuminant)
        matrix = colour.RGB_to_XYZ(
            np.eye(3),
            "ACEScg",
            illuminant=xy,
            chromatic_adaptation_transform="CAT16",
            apply_cctf_decoding=False,
        ).T
        illuminant = p.film.info.reference_illuminant
        if illuminant not in catalog["remap"]:
            compressed = gamut.compress_xy(
                spectral._quad2tri(tc_cells).reshape(-1, 2), xy, p.io.input_gamut_compress
            ).reshape(n, n, 2)
            coords = spectral._tri2quad(compressed).astype("<f8")
            file = f"input_gamut_{illuminant}.f64.z"
            (output / file).write_bytes(zlib.compress(coords.tobytes(), 9))
            catalog["remap"][illuminant] = file
        valid = np.isfinite(d.base_density) & np.isfinite(d.channel_density).all(axis=1)
        catalog["negative"].append(
            {
                "id": name,
                "name": p.film.info.name,
                "illuminant": illuminant,
                "matrix": matrix.tolist(),
                "sensitivity": effective.tolist(),
                "log_exposure": d.log_exposure.tolist(),
                "density_curves": curve.tolist(),
                "pre_DIR_curves": pre.tolist(),
                "DIR_matrix": M.tolist(),
                "gamma": np.broadcast_to(p.film_render.density_curve_gamma, (3,)).tolist(),
                "dyes": np.nan_to_num(d.channel_density).tolist(),
                "base": np.nan_to_num(d.base_density).tolist(),
                "support": valid.tolist(),
            }
        )
    probes = np.array(
        [
            [0.18, 0.18, 0.18],
            [0.18, 0.02, 0.01],
            [0.02, 0.18, 0.01],
            [0.01, 0.02, 0.18],
            [0.32, 0.16, 0.10],
            [-0.01, 0.12, 0.3],
            [2.0, 2.0, 2.0],
            [0.0, 0.0, 0.0],
        ]
    )
    fixtures = {"revision": REVISION, "cases": [], "paper_only": []}
    for name in paper_ids:
        p = params("kodak_portra_400", name)
        pipe = Pipeline(p)
        stage = pipe._printing_stage
        scanner = pipe._scanning_stage
        axis = np.asarray(p.print.data.log_exposure)
        curves = stage.develop(np.repeat(axis[:, None], 3, axis=1).reshape(1, -1, 3)).reshape(-1, 3)

        def read_h(h):
            return scanner.scan(stage.develop(np.array(h).reshape(1, -1, 3))).reshape(-1, 3)

        initial = stage.expose(
            pipe._filming_stage.develop(pipe._filming_stage.expose(np.full((1, 1, 3), 0.18)))
        ).reshape(3)
        result = optimize.least_squares(
            lambda h: read_h(h)[0] - gray_target,
            initial,
            bounds=(float(axis.min()), float(axis.max())),
            xtol=1e-13,
            ftol=1e-13,
            gtol=1e-13,
        )
        assert result.success and np.max(abs(read_h(result.x)[0] - gray_target)) < 1e-9, (
            name,
            result.message,
        )
        nscan = args.scanner_size
        lo = curves.min(axis=0)
        hi = curves.max(axis=0)
        axes = [np.linspace(a, b, nscan) for a, b in zip(lo, hi)]
        density = np.stack(np.meshgrid(*axes, indexing="ij"), axis=-1).reshape(-1, 3)
        rgb = np.concatenate(
            [
                scanner.scan(v.reshape(1, -1, 3)).reshape(-1, 3)
                for v in np.array_split(density, max(1, len(density) // 8192))
            ]
        )
        encoded = cct(ocio(reference_rgb(rgb), "inverse"))
        filename = f"{name}-scan.cct.z"
        (output / filename).write_bytes(zlib.compress(encoded.astype("<f4").tobytes(), 9))
        catalog["paper"].append(
            {
                "id": name,
                "name": p.print.info.name,
                "log_exposure": axis.tolist(),
                "density_curves": curves.tolist(),
                "scanner_min": lo.tolist(),
                "scanner_max": hi.tolist(),
                "scanner_file": filename,
                "positive_gray_logH": result.x.tolist(),
            }
        )
        paper_h = result.x - 0.6 * 17.52 * (cct(probes) - cct(np.full((1, 3), 0.18))) * np.log10(2)
        out = read_h(paper_h)
        density = stage.develop(paper_h.reshape(1, -1, 3)).reshape(-1, 3)
        fixtures["paper_only"].append(
            {
                "paper": name,
                "input": probes.tolist(),
                "paper_h": paper_h.tolist(),
                "paper_density": density.tolist(),
                "reference_rgb": reference_rgb(out).tolist(),
            }
        )
        for film in negative_ids:
            p = params(film, name)
            pipe = Pipeline(p)
            filming = pipe._filming_stage
            stage = pipe._printing_stage
            scanner = pipe._scanning_stage
            neutral = filming.develop(filming.expose(np.full((1, 1, 3), 0.18)))
            pre_h = stage._film_cmy_to_print_log_raw(neutral)

            def gray_at(logtime):
                h = np.log10(10**pre_h * 10**logtime + 1e-10)
                out = scanner.scan(stage.develop(h)).reshape(3)
                return float(out @ srgb_xyz[1]) - 0.1

            logtime = optimize.brentq(gray_at, -4, 4, xtol=1e-13)
            p.enlarger.print_exposure = 10**logtime
            pipe = Pipeline(p)
            filming = pipe._filming_stage
            stage = pipe._printing_stage
            scanner = pipe._scanning_stage
            sensitivity = np.nan_to_num(10**p.print.data.log_sensitivity)
            source = standard_illuminant(p.enlarger.illuminant)
            lamp = stage._enlarger_service.enlarger_filtered_illuminant(source)
            factor = float(
                np.asarray(stage._compute_exposure_factor_midgray(sensitivity, lamp)).reshape(-1)[0]
            )
            preflash = stage._compute_raw_preflash(source, sensitivity).reshape(3)
            catalog["pairs"].append(
                {
                    "negative": film,
                    "paper": name,
                    "weights": (lamp[:, None] * sensitivity * factor).tolist(),
                    "preflash": preflash.tolist(),
                    "print_exposure": p.enlarger.print_exposure,
                }
            )
            h = filming.expose(probes.reshape(1, -1, 3))
            d = filming.develop(h)
            ph = stage.expose(d)
            pd = stage.develop(ph)
            out = scanner.scan(pd).reshape(-1, 3)
            fixtures["cases"].append(
                {
                    "negative": film,
                    "paper": name,
                    "input": probes.tolist(),
                    "film_h": h.reshape(-1, 3).tolist(),
                    "film_density": d.reshape(-1, 3).tolist(),
                    "paper_h": ph.reshape(-1, 3).tolist(),
                    "paper_density": pd.reshape(-1, 3).tolist(),
                    "reference_rgb": reference_rgb(out).tolist(),
                }
            )
        print(name, "complete", flush=True)
    raw = json.dumps(catalog, separators=(",", ":"), allow_nan=False).encode()
    (output / "catalog.json.z").write_bytes(zlib.compress(raw, 9))
    (output / "conformance.json").write_text(
        json.dumps(fixtures, separators=(",", ":"), allow_nan=False) + "\n"
    )
    selection = {
        "revision": REVISION,
        **{
            key: [{"id": v["id"], "name": v["name"]} for v in catalog[key]]
            for key in ["negative", "paper"]
        },
    }
    (output / "selection.json").write_text(json.dumps(selection, indent=2) + "\n")
    entries = [f'    ("{p.name}", include_bytes!("{p.name}")),' for p in sorted(output.glob("*.z"))]
    (output / "index.rs").write_text(
        "// Generated by scripts/spektrafilm/export.py.\npub(super) const FILES: &[(&str, &[u8])] = &[\n"
        + "\n".join(entries)
        + "\n];\n"
    )
    sources = {
        str(p.relative_to(upstream)): sha(p)
        for p in sorted((upstream / "src/spektrafilm").rglob("*.py"))
    }
    provenance = {
        "source": "https://github.com/andreavolpato/spektrafilm",
        "author": "Andrea Volpato",
        "revision": REVISION,
        "modification": "Antitone evaluates and factors the nonspatial model, calibrates print time on neutral0.18 toY0.1, adds the positive-paper adapter and inverse authoring DRT; no photographic fitting. Data derivatives CC BY-SA4.0; native algorithm ports GPLv3.",
        "source_code_sha256": sources,
        "environment_lock_sha256": sha(LOCK),
        "build_tool_sha256": sha(ROOT / "scripts/spektrafilm/build.py"),
        "authoring_reference_sha256": sha(ROOT / "assets/ocio/authoring-reference.json"),
        "locked_dependencies": dependencies,
        "generation_dependencies": {
            "python": sys.version,
            "numpy": np.__version__,
            "scipy": importlib.import_module("scipy").__version__,
            "colour-science": colour.__version__,
            "OpenColorIO": "2.6.0",
        },
        "generator_sha256": sha(Path(__file__)),
        "reference_helper_sha256": sha(ROOT / "native/spektrafilm/reference.cpp"),
        "source_lut_sha256": sha(
            upstream / "src/spektrafilm/data/luts/spectral_upsampling/irradiance_xy_tc.npy"
        ),
        "files": {
            p.name: {"bytes": p.stat().st_size, "sha256": sha(p)} for p in sorted(output.iterdir())
        },
    }
    (output / "provenance.json").write_text(json.dumps(provenance, indent=2) + "\n")
    print("Runtime asset bytes", sum(p.stat().st_size for p in output.glob("*.z")))


if __name__ == "__main__":
    main()
