"""Source evaluation of the shared reference-negative characteristic curve."""

import json
import math
from pathlib import Path

DEFINITION = Path(__file__).resolve().parents[2] / "assets/looks/reference-negative.json"
PARAMETERS = json.loads(DEFINITION.read_text())


def density(signal):
    if signal <= 0:
        return 0.0
    z = PARAMETERS["toe"] * (math.log(signal) - math.log(PARAMETERS["knee"]))
    softplus = max(z, 0.0) + math.log1p(math.exp(-abs(z)))
    return softplus * PARAMETERS["gamma"] / (PARAMETERS["toe"] * math.log(10))
