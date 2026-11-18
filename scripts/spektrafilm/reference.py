"""Paper-only exposure basis and effective dyes; independent scalar evaluation."""

import hashlib
import json
import math
from pathlib import Path

if __package__:
    from .characteristic import density
else:
    from characteristic import density

ROOT = Path(__file__).resolve().parents[2]
DEFINITION = ROOT / "assets/looks/paper-reference.json"
CALIBRATION = ROOT / "assets/looks/paper-reference-calibration.json"
EPSILON = 1e-10


def definition():
    value = json.loads(DEFINITION.read_text())
    if (
        set(value)
        != {
            "schema",
            "negative",
            "exposure_rows",
            "dye_rows",
            "density_gain",
            "base_density_offset",
        }
        or value["schema"] != 1
        or not value["negative"]
    ):
        raise ValueError("invalid paper-reference definition")
    for key in ["exposure_rows", "dye_rows"]:
        rows = value[key]
        if len(rows) != 3 or any(
            len(row) != 3 or not all(math.isfinite(v) for v in row) or abs(sum(row) - 1) > 1e-12
            for row in rows
        ):
            raise ValueError("invalid neutral-preserving paper-reference matrix")
        a, b, c = rows
        determinant = (
            a[0] * (b[1] * c[2] - b[2] * c[1])
            - a[1] * (b[0] * c[2] - b[2] * c[0])
            + a[2] * (b[0] * c[1] - b[1] * c[0])
        )
        if abs(determinant) <= 1e-10:
            raise ValueError("singular paper-reference matrix")
    if not math.isfinite(value["density_gain"]) or value["density_gain"] <= 0:
        raise ValueError("invalid paper-reference density gain")
    if not math.isfinite(value["base_density_offset"]) or value["base_density_offset"] < 0:
        raise ValueError("invalid paper-reference base offset")
    return value


NEGATIVE = definition()["negative"]


def multiply(rows, vector):
    return [sum(a * b for a, b in zip(row, vector)) for row in rows]


class Reference:
    """Peak-OD coordinates in the retained signed spectral basis, not concentrations."""

    def __init__(self, film, pair):
        self.definition = definition()
        if film["id"] != self.definition["negative"] or pair["negative"] != film["id"]:
            raise ValueError("wrong paper-reference spectral basis")
        self.pair = pair
        self.support = film["support"]
        peaks = [
            max(d[c] for d, valid in zip(film["dyes"], self.support) if valid) for c in range(3)
        ]
        if min(peaks) <= 0:
            raise ValueError("invalid paper-reference dye peaks")
        self.dyes = [[v / peak for v, peak in zip(d, peaks)] for d in film["dyes"]]
        self.base = film["base"]

    def amounts(self, rgb):
        records = multiply(self.definition["exposure_rows"], rgb)
        developed = [density(v) for v in records]
        return [
            self.definition["density_gain"] * v
            for v in multiply(self.definition["dye_rows"], developed)
        ]

    def transmission(self, amounts):
        offset = self.definition["base_density_offset"]
        return [
            10 ** -(base + offset + sum(a * d for a, d in zip(amounts, dyes))) if valid else 0.0
            for base, dyes, valid in zip(self.base, self.dyes, self.support)
        ]

    def expose(self, rgb):
        transmission = self.transmission(self.amounts(rgb))
        lamp_scale = 10 ** self.definition["base_density_offset"]
        raw = [
            sum(w[c] * lamp_scale * t for w, t in zip(self.pair["weights"], transmission))
            + self.pair["preflash"][c]
            for c in range(3)
        ]
        return [
            math.log10((max(v, 0) + EPSILON) * self.pair["print_exposure"] + EPSILON) for v in raw
        ]


def source():
    definition_hash = hashlib.sha256(DEFINITION.read_bytes()).hexdigest()
    calibration = json.loads(CALIBRATION.read_text())
    characteristic = ROOT / "assets/looks/reference-negative.json"
    if (
        calibration["definition_sha256"] != definition_hash
        or calibration["characteristic_sha256"]
        != hashlib.sha256(characteristic.read_bytes()).hexdigest()
    ):
        raise ValueError("paper-reference calibration does not describe this definition")
    return {
        "definition_sha256": definition_hash,
        "calibration_sha256": hashlib.sha256(CALIBRATION.read_bytes()).hexdigest(),
        "negative": NEGATIVE,
        "formation": "A AP1 chooses exposure records; shared F develops them; g B maps into peak-normalized effective dye coordinates. Source base and signed spectra retained.",
        "printing": "Fixed neutral filtration with reciprocal lamp scaling; fixed per-medium exposure/balance at AP1 gray. No per-input or photographic calibration.",
        "physical_scope": "Passive spectral-filter realization over the authored input cube; not negative-film chemistry or actual-negative recovery.",
    }
