#!/usr/bin/env python3
"""Propose a smooth B&W characteristic response from retained graph readings.

Authoring only: writes a fresh candidate for review, never updates the catalogue
or refits saved reconstructions during component generation. Standard library only.
"""

import argparse
import copy
from html import escape
import json
import math
from pathlib import Path

if __package__:
    from . import monochrome
else:
    import monochrome

# A bounded coarse-to-fine search, not a physical material-bandwidth estimate.
INTERVALS = (6, 8, 10, 12, 16, 24)


def project_monotone(values, low, high):
    """Least-squares projection onto bounded nondecreasing control values."""
    blocks = []
    for value in values:
        blocks.append((value, 1))
        while len(blocks) > 1 and blocks[-2][0] / blocks[-2][1] > blocks[-1][0] / blocks[-1][1]:
            right, left = blocks.pop(), blocks.pop()
            blocks.append((left[0] + right[0], left[1] + right[1]))
    return [min(high, max(low, total / count)) for total, count in blocks for _ in range(count)]


def fit_controls(samples, domain, intervals):
    """Projected accelerated least squares, with the two endpoints fixed."""
    count = intervals + 3
    normal = [[0.0] * count for _ in range(count)]
    target = [0.0] * count
    # Unit controls expose the existing spline evaluator's linear basis. This
    # keeps fitting and generation on one definition of the response family.
    units = [
        {"intervals": intervals, "control_values": [float(i == j) for j in range(count)]}
        for i in range(count)
    ]
    for log_h, density in samples:
        basis = [monochrome.spline_density(unit, domain, log_h) for unit in units]
        nonzero = [(i, value) for i, value in enumerate(basis) if value]
        for i, value in nonzero:
            target[i] += value * density / len(samples)
            for j, other in nonzero:
                normal[i][j] += value * other / len(samples)
    step = 1 / max(sum(abs(v) for v in row) for row in normal)
    low, high = samples[0][1], samples[-1][1]
    # Greville abscissae reproduce a straight line even when the retained trace
    # has too few points to determine every control. Index-spaced controls would
    # invent endpoint curvature in that underdetermined case.
    knots = [0.0] * 4 + [i / intervals for i in range(1, intervals)] + [1.0] * 4
    controls = [low + (high - low) * sum(knots[i + 1 : i + 4]) / 3 for i in range(count)]
    extrapolated, momentum = controls[:], 1.0
    resolution = 1e-11 * max(1.0, high - low)
    for iteration in range(12000):
        trial = [
            extrapolated[i]
            - step * (sum(normal[i][j] * extrapolated[j] for j in range(count)) - target[i])
            for i in range(count)
        ]
        updated = [low, *project_monotone(trial[1:-1], low, high), high]
        if max(abs(a - b) for a, b in zip(updated, controls)) < resolution:
            return updated, iteration + 1
        next_momentum = (1 + math.sqrt(1 + 4 * momentum * momentum)) / 2
        extrapolated = [
            a + (momentum - 1) / next_momentum * (a - b) for a, b in zip(updated, controls)
        ]
        controls, momentum = updated, next_momentum
    raise ValueError(f"{intervals}-interval characteristic fit did not converge")


def reconstruct(source):
    if source["reconstruction"]["family"] != "tabulated":
        raise ValueError(
            "reconstruction requires an unfitted tabulated trace; existing fits stay fixed"
        )
    monochrome.audit_source(source)
    samples = source["reconstruction"]["samples"]
    domain = source["material"]["log_exposure_range"]
    allowance = source["reading_uncertainty"]["density"]
    if (
        len(domain) != 2
        or any(not math.isfinite(v) for v in domain)
        or domain[0] >= domain[1]
        or not math.isfinite(domain[1] - domain[0])
        or [samples[0][0], samples[-1][0]] != domain
        or not math.isfinite(allowance)
        or allowance <= 0
    ):
        raise ValueError("invalid trace domain or density-reading allowance")
    axis = [domain[0] + (domain[1] - domain[0]) * i / 2048 for i in range(2049)]
    attempts = []
    for intervals in INTERVALS:
        controls, iterations = fit_controls(samples, domain, intervals)
        candidate = copy.deepcopy(source)
        reconstruction = candidate["reconstruction"]
        if "method" in reconstruction:
            reconstruction.setdefault("trace_method", reconstruction["method"])
        reconstruction.update(
            family="monotone-cubic-bspline",
            intervals=intervals,
            control_values=controls,
            method="Least-squares clamped uniform cubic B-spline fitted to the retained samples "
            "with nondecreasing controls and fixed endpoints; first coarse-to-fine candidate "
            "within the recorded density-reading allowance. Samples retain the graph trace, "
            "not interpolation knots.",
        )
        change = max(
            abs(monochrome.source_density(candidate, h) - monochrome.source_density(source, h))
            for h in [*axis, *(p[0] for p in samples)]
        )
        distances = []
        for point in source["constraints"]:
            density = monochrome.source_density(candidate, point["log_exposure"])
            distances.append(
                min(max(lo - density, 0.0, density - hi) for lo, hi in point["coverage_intervals"])
            )
        outside = max(distances)
        attempts.append(
            {
                "intervals": intervals,
                "iterations": iterations,
                "max_density_change_from_trace": change,
                "max_density_outside_ink_interval": outside,
            }
        )
        if change <= allowance and outside <= allowance:
            audit = monochrome.audit_source(candidate)
            return candidate, {
                "material": source["material"]["id"],
                "density_reading_allowance": allowance,
                "attempts": attempts,
                "source_audit": audit,
            }
    raise ValueError(
        "no coarse reconstruction fits the readings; inspect the source rather than "
        "loosening its uncertainty: " + json.dumps(attempts)
    )


def overlay(source, candidate):
    domain = source["material"]["log_exposure_range"]
    samples = source["reconstruction"]["samples"]
    allowance = source["reading_uncertainty"]["density"]
    bottom, top = samples[0][1] - allowance, samples[-1][1] + allowance

    def points(values):
        return " ".join(
            f"{50 + 800 * (h - domain[0]) / (domain[1] - domain[0]):.3f},"
            f"{330 - 270 * (d - bottom) / (top - bottom):.3f}"
            for h, d in values
        )

    band = [(h, d - allowance) for h, d in samples]
    band += [(h, d + allowance) for h, d in reversed(samples)]
    fitted = [
        (h, monochrome.source_density(candidate, h))
        for h in [domain[0] + (domain[1] - domain[0]) * i / 2048 for i in range(2049)]
    ]
    label = escape(source["material"]["name"])
    return (
        '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 900 380">'
        '<rect width="900" height="380" fill="white"/>'
        f'<text x="50" y="25" font-family="sans-serif" font-size="14">{label}</text>'
        f'<polygon fill="#eee" points="{points(band)}"/>'
        f'<polyline fill="none" stroke="#666" points="{points(samples)}"/>'
        f'<polyline fill="none" stroke="#d65b00" points="{points(fitted)}"/>'
        '<text x="50" y="355" font-family="sans-serif" font-size="12">'
        "Log exposure → / density ↑; gray: retained trace ± density allowance; orange: fit</text>"
        "</svg>\n"
    )


def propose(source_path, output):
    output = output.resolve()
    if output.exists() or not output.is_relative_to((monochrome.ROOT / ".cache").resolve()):
        raise ValueError("choose a fresh output directory under checkout .cache/")
    source = json.loads(source_path.read_text(encoding="utf-8"))
    candidate, audit = reconstruct(source)
    payloads = {
        "source.json": json.dumps(candidate, indent=2, ensure_ascii=False) + "\n",
        "audit.json": json.dumps(audit, indent=2) + "\n",
        "curve.svg": overlay(source, candidate),
    }
    output.mkdir(parents=True)
    for name, content in payloads.items():
        (output / name).write_text(content, encoding="utf-8")
    return audit


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path, help="new material's tabulated source record")
    parser.add_argument(
        "--output", type=Path, required=True, help="fresh candidate directory in .cache/"
    )
    args = parser.parse_args()
    try:
        audit = propose(args.source, args.output)
    except (ValueError, KeyError, OSError) as error:
        parser.error(str(error))
    print(json.dumps(audit, indent=2))
    print(
        f"Review {args.output}/curve.svg and the generated photographic gradients before promotion."
    )


if __name__ == "__main__":
    main()
