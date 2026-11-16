"""Reference-negative input with fixed per-medium printing calibration."""

from characteristic import PARAMETERS
from reference import Reference, prepare as prepare_reference, source as reference_source


def prepare(profile, negative, responsivities, gray_log_h):
    """Calibrate printing exposure/balance once, through the full reference branch."""
    import colour
    import numpy as np

    driver = {
        "input_luminance": colour.RGB_COLOURSPACES["ACEScg"].matrix_RGB_to_XYZ[1].tolist(),
        "reference": prepare_reference(profile, negative, responsivities),
        "reference_source": reference_source(responsivities),
        "intent": "Shared reference-negative characteristic curve and Academy density basis; convex spectrum selection with fixed base prior, adapter-only cone projection, and fixed printing calibration at AP1 gray 0.18. No per-input neutral subtraction or photographic fitting.",
    }
    reference = Reference(driver["reference"])
    gray = PARAMETERS["gray"]
    driver["print_log_h"] = (
        np.array(gray_log_h) - reference.expose(np.full(3, gray), gray)
    ).tolist()
    return driver


def expose(inputs, driver):
    """Independent SciPy reference moments plus fixed printing exposure/balance."""
    import numpy as np

    reference = Reference(driver["reference"])
    luminance = np.array(driver["input_luminance"])
    printing = np.array(driver["print_log_h"])
    return np.array(
        [printing + reference.expose(rgb, luminance @ rgb) for rgb in np.atleast_2d(inputs)]
    )
