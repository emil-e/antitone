"""Source-model checks, independent of the frozen runtime representation.
Baking never refits curves or regenerates frozen coefficient tables. These
checks connect those tables to numerical observations and published equations.
"""

import math
from catalog import common, require


def characteristic(p, h):
    z = -p[2] * (h - p[3])
    softplus = z + math.log1p(math.exp(-z)) if z > 0 else math.log1p(math.exp(z))
    signal = math.exp(-p[4] * softplus)
    if len(p) == 8:
        z = -p[6] * (h - p[7])
        sigmoid = math.exp(-z) / (1 + math.exp(-z)) if z > 0 else 1 / (1 + math.exp(z))
        signal = (1 - p[5]) * signal + p[5] * sigmoid
    return p[0] + (p[1] - p[0]) * signal


class Curve:
    def __init__(self, x, y):
        self.x, self.y = x, y
        h = [b - a for a, b in zip(x, x[1:])]
        d = [(b - a) / step for a, b, step in zip(y, y[1:], h)]
        self.m = [d[0]]
        for i in range(1, len(x) - 1):
            if d[i - 1] * d[i] <= 0:
                self.m.append(0.0)
            else:
                a, b = 2 * h[i] + h[i - 1], h[i] + 2 * h[i - 1]
                self.m.append((a + b) / (a / d[i - 1] + b / d[i]))
        self.m.append(d[-1])

    def __call__(self, v):
        if v <= self.x[0]:
            return self.y[0]
        if v >= self.x[-1]:
            return self.y[-1]
        i = next(i for i in range(len(self.x) - 1) if self.x[i] <= v <= self.x[i + 1])
        h = self.x[i + 1] - self.x[i]
        t = (v - self.x[i]) / h
        return (
            (2 * t**3 - 3 * t * t + 1) * self.y[i]
            + (t**3 - 2 * t * t + t) * h * self.m[i]
            + (-2 * t**3 + 3 * t * t) * self.y[i + 1]
            + (t**3 - t * t) * h * self.m[i + 1]
        )


def spectral_product(definition, wavelength):
    points = definition["points"]
    if wavelength < points[0][0]:
        value = (
            points[0][1]
            + (wavelength - points[0][0]) * definition["short_wavelength_log_slope_per_nm"]
        )
    elif wavelength > points[-1][0]:
        value = (
            points[-1][1]
            + (wavelength - points[-1][0]) * definition["long_wavelength_log_slope_per_nm"]
        )
    else:
        i = next(
            i for i in range(len(points) - 1) if points[i][0] <= wavelength <= points[i + 1][0]
        )
        a, va = points[i]
        b, vb = points[i + 1]
        value = va + (vb - va) * (wavelength - a) / (b - a)
    return 10 ** (value - 5)


def common_equations(spectral):
    terms = [
        [
            (0.362, 442.0, 0.0624, 0.0374),
            (1.056, 599.8, 0.0264, 0.0323),
            (-0.065, 501.1, 0.049, 0.0382),
        ],
        [(0.821, 568.8, 0.0213, 0.0247), (0.286, 530.9, 0.0613, 0.0322)],
        [(1.217, 437.0, 0.0845, 0.0278), (0.681, 459.0, 0.0385, 0.0725)],
    ]

    def planck(w):
        wavelength = w * 1e-9
        return wavelength**-5 / math.expm1(0.014387768775039337 / (wavelength * 5000.0))

    for i, w in enumerate(spectral["wavelengths"]):
        observer = [
            sum(
                a * math.exp(-0.5 * ((w - center) * (left if w < center else right)) ** 2)
                for a, center, left, right in channel
            )
            for channel in terms
        ]
        illumination = planck(w) / planck(560.0) * (0.5 if i in (0, 60) else 1.0)
        require(
            max(abs(a - b) for a, b in zip(observer, spectral["observer"][i])) < 1e-14,
            "shared observer differs from published equations",
        )
        require(
            abs(illumination - spectral["illumination"][i]) < 1e-14,
            "shared illumination differs from Planck convention",
        )


def validate(definition, assets=None):
    authoring, spectral, status = common() if assets is None else common(assets)
    common_equations(spectral)
    source, model = definition.source, definition.model
    params = source["reconstruction"]["parameters"]

    def distance(value, span):
        return max(0.0, span[0] - value, value - span[1])

    source_error = coverage_error = 0.0
    for row in source["constraints"]:
        values = [characteristic(p, row["log_exposure"]) for p in params]
        source_error = max(
            source_error,
            max(distance(v, span) for v, span in zip(values, row["channel_intervals"])),
        )
        if not definition.monochrome:
            coverage_error = max(
                coverage_error,
                max(
                    (min(distance(v, span) for v in values) for span in row["coverage_intervals"]),
                    default=0.0,
                ),
            )
    require(
        max(source_error, coverage_error) <= authoring["source_density_allowance"],
        "original source-interval admission failed",
    )
    result = {"source_interval_error": source_error, "source_coverage_error": coverage_error}
    if definition.monochrome:
        require(params == [model["density_parameters"]], "scalar source/runtime parameters differ")
        return result
    readings = source["dye_readings"]
    curves = []
    for name in ("cyan", "magenta", "yellow"):
        rows = readings[name]
        require([r["nm"] for r in rows] == list(range(400, 701, 10)), "unexpected dye observations")
        require(
            all(math.isfinite(r["density"]) and r["density"] >= 0 for r in rows),
            "invalid dye reading",
        )
        curves.append(Curve([r["nm"] for r in rows], [r["density"] for r in rows]))
    dyes = [[curve(w) for curve in curves] for w in spectral["wavelengths"]]
    require(
        max(
            abs(v - u)
            for row, frozen in zip(dyes, model["dye_density"])
            for v, u in zip(row, frozen)
        )
        < 1e-13,
        "source/runtime dye spectra differ",
    )
    meters = []
    for channel in ("red", "green", "blue"):
        weights = [
            spectral_product(status[channel], w) * (0.5 if i in (0, 60) else 1.0)
            for i, w in enumerate(spectral["wavelengths"])
        ]
        total = sum(weights)
        meters.append([v / total for v in weights])
    error = 0.0
    for h, amounts in zip(model["log_exposure"], model["effective_coefficients"]):
        attenuation = [10 ** (-sum(a * d for a, d in zip(amounts, row))) for row in dyes]
        meter = [
            -math.log10(sum(w * r for w, r in zip(weights, attenuation))) for weights in meters
        ]
        target = [characteristic(p, h) for p in params]
        error = max(error, max(abs(v - u) for v, u in zip(meter, target)))
    require(error < 1e-8, "frozen effective coefficients no longer reconstruct source densities")
    result["meter_density_error"] = error
    return result
