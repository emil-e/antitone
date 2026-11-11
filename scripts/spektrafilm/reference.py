"""Historical Academy flux reference and a minimum-relative-entropy spectrum."""

from bisect import bisect_right
import csv
import hashlib
from pathlib import Path
import struct

NEGATIVE = "kodak_vision3_250d"
APD_NUMERIC_SHA256 = "50bfb192b467f41a6903e44c4d4f15bb5413937f8b94b0696330019d37952f1f"


def responsivities(path):
    """Read a separately supplied, canonically identified historical table."""
    with Path(path).open(newline="") as stream:
        reader = csv.reader(stream)
        next(reader, None)
        values = [[float(v) for v in row] for row in reader]
    if len(values) != 186 or any(len(row) != 4 for row in values):
        raise ValueError("invalid APD responsivity table shape")
    numeric = b"".join(struct.pack("<4d", *row) for row in values)
    if [row[0] for row in values] != list(range(360, 731, 2)) or hashlib.sha256(
        numeric
    ).hexdigest() != APD_NUMERIC_SHA256:
        raise ValueError("supply the reviewed S-2008-002 v1.1 APD responsivity table")
    return values


def receiver_weights(knots, prior_knots, wavelengths, prior_values, action):
    """Positive relative-filter interpolation with constant endpoint extension."""
    weights = [[0.0, 0.0, 0.0] for _ in knots]
    indices = [min(max(bisect_right(knots, w) - 1, 0), len(knots) - 2) for w in wavelengths]
    fractions = [
        min(max((w - knots[i]) / (knots[i + 1] - knots[i]), 0.0), 1.0)
        for w, i in zip(wavelengths, indices)
    ]
    for c in range(3):
        for i, f, t, a in zip(indices, fractions, prior_values, action):
            weights[i][c] += a[c] * t * (1 - f) / prior_knots[i]
        for i, f, t, a in zip(indices, fractions, prior_values, action):
            weights[i + 1][c] += a[c] * t * f / prior_knots[i + 1]
    return weights


def prepare(paper, negative, path):
    import colour
    import numpy as np
    from spektrafilm.config import SPECTRAL_SHAPE
    from spektrafilm.model.illuminants import standard_illuminant

    table = np.array(responsivities(path))
    action = table[:, 1:] / table[:, 1:].sum(axis=0)
    data = negative.data
    grid = SPECTRAL_SHAPE.wavelengths
    support = np.isfinite(data.channel_density).all(axis=1) & np.isfinite(data.base_density)
    measured = grid[support]
    active = (table[:, 0] >= measured[0]) & (table[:, 0] <= measured[-1]) & (action.sum(axis=1) > 0)
    knots = table[active, 0]
    action = action[active]
    prior = 10 ** -np.interp(knots, measured, data.base_density[support])
    total_action = action.sum(axis=1)
    mass = total_action * prior
    mass /= mass.sum()
    responses = action / total_action[:, None]
    wavelengths = grid[support]
    paper_action = standard_illuminant("TH-KG3")[:, None] * np.nan_to_num(
        10**paper.data.log_sensitivity, nan=0.0
    )
    prior_paper = 10 ** -data.base_density[support]
    receiver = np.array(
        receiver_weights(knots, prior, wavelengths, prior_paper, paper_action[support])
    )
    receiver /= total_action[:, None]
    ap1 = colour.RGB_COLOURSPACES["ACEScg"]
    ap0 = colour.RGB_COLOURSPACES["ACES2065-1"]
    exposure = np.array(
        [[0.72286, 0.12630, 0.15084], [0.11923, 0.76418, 0.11659], [0.01427, 0.08213, 0.90359]]
    )
    return {
        "to_records": np.linalg.solve(
            exposure, ap0.matrix_XYZ_to_RGB @ ap1.matrix_RGB_to_XYZ
        ).tolist(),
        "flux0": (action * prior[:, None]).sum(axis=0).tolist(),
        "samples": [
            {"response": r.tolist(), "log_prior": float(np.log(p)), "receiver": w.tolist()}
            for r, p, w in zip(responses, mass, receiver)
        ],
    }


def source(path):
    return {
        "reference": "S-2008-002 v1.1, November 18, 2010; APD and ADX gains (1, .92, .95)",
        "url": "https://github.com/aces-aswf/aces-core/blob/v1.0.3/documents/LaTeX/TB-2014-005/S-2008-002.pdf",
        "numeric_sha256": APD_NUMERIC_SHA256,
        "input_sha256": hashlib.sha256(Path(path).read_bytes()).hexdigest(),
        "prior_and_Dmin": NEGATIVE + " base; no negative characteristic curves",
        "selector": "Minimum relative entropy of summed APD-action distribution at fixed flux ratios; canonical mass fixes total flux.",
        "receiver": "TH-KG3; positive relative-filter interpolation with held endpoints, opaque outside supplied common negative support.",
        "scale": "Canonical transmission may exceed one. Passive-filter/common-printer-exposure representation is an output-invariant gauge, not identical fixed-scale filter APD readings.",
        "distribution": "Private trial; public redistribution permission for APD reference data remains unestablished.",
    }


class Reference:
    """Independent SciPy-root observations of the convex spectral constraints."""

    def __init__(self, data):
        import numpy as np

        self.np = np
        self.matrix = np.array(data["to_records"])
        self.flux0 = np.array(data["flux0"])
        self.responses = np.array([s["response"] for s in data["samples"]]).T
        self.log_prior = np.array([s["log_prior"] for s in data["samples"]])
        self.receiver = np.array([s["receiver"] for s in data["samples"]]).T
        self.prior_response = self.responses @ np.exp(self.log_prior)
        self.inverse_cdd = np.linalg.inv(
            [[0.75573, 0.22197, 0.02230], [0.05901, 0.96928, -0.02829], [0.16134, 0.07406, 0.76460]]
        )
        self.gain = np.array([1.0, 0.92, 0.95])

    def target(self, records):
        np = self.np
        cid = np.array([-0.19, 0.01, 0.028, 0.054, 0.095, 0.145, 0.22, 0.3, 0.4, 0.5, 0.6])
        loge = np.array(
            [
                -6.0,
                -2.721718645,
                -2.521718645,
                -2.321718645,
                -2.121718645,
                -1.921718645,
                -1.721718645,
                -1.521718645,
                -1.321718645,
                -1.121718645,
                -0.926545676714876,
            ]
        )
        log = np.log10(records)
        k = np.clip(np.searchsorted(loge, log, side="right") - 1, 0, 9)
        slope = np.where(log > loge[-1], 0.55, (cid[k + 1] - cid[k]) / (loge[k + 1] - loge[k]))
        value = np.where(
            log > loge[-1], 0.55 * (log + 0.7 / 0.55 - np.log10(0.18)), np.interp(log, loge, cid)
        )
        return self.inverse_cdd @ value / self.gain, slope / (records * np.log(10))

    def field(self, records):
        from scipy.optimize import root
        from scipy.special import logsumexp

        np = self.np
        anchor = np.maximum(records, 1 / 128)
        target, slope = self.target(anchor)
        log_flux = np.log(self.flux0) - np.log(10) * target
        log_total = logsumexp(log_flux)
        q = np.exp(log_flux - log_total)
        omitted = int(np.argmax(q))
        kept = np.flatnonzero(np.arange(3) != omitted)
        features = self.responses[kept]

        def moments(theta):
            values = self.log_prior + theta @ features
            p = np.exp(values - logsumexp(values))
            mean = features @ p
            covariance = (features * p) @ features.T - np.outer(mean, mean)
            return p, mean, covariance

        initial = np.log(q[kept] / q[omitted]) - np.log(
            self.prior_response[kept] / self.prior_response[omitted]
        )
        solution = root(
            lambda t: moments(t)[1] - q[kept], initial, jac=lambda t: moments(t)[2], tol=1e-12
        )
        p, mean, covariance = moments(solution.x)
        residual = np.max(abs(np.log(self.responses @ p) - np.log(q)))
        if not np.isfinite(p).all() or residual > 1e-11:
            raise ValueError(f"reference moment observation failed: {residual}")
        flux = self.receiver @ p
        delta_log_flux = -np.log(10) * (self.inverse_cdd @ (slope * (records - anchor))) / self.gain
        delta_total = q @ delta_log_flux
        delta_q = q[kept] * (delta_log_flux[kept] - delta_total)
        delta_theta = np.linalg.solve(covariance, delta_q)
        receiver_mean = (self.receiver * p) @ features.T / flux[:, None]
        return (
            log_total + np.log(flux) + delta_total + (receiver_mean - mean) @ delta_theta
        ) / np.log(10)

    def difference(self, rgb, brightness):
        np = self.np
        if brightness <= 0:
            return np.zeros(3)
        r = self.matrix @ rgb
        n = self.matrix @ np.full(3, brightness)
        if min(r) < 0:
            distance = np.max((n - r) / n)
            r = n + (r - n) / distance
        return self.field(r) - self.field(n)
