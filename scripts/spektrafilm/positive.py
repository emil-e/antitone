"""Reference-negative input with calibrated neutral exposure."""

from reference import Reference, prepare as prepare_reference, source as reference_source


def prepare(profile, negative, responsivities):
    """Export the convex spectral selector and working luminance convention."""
    import colour

    return {
        "input_luminance": colour.RGB_COLOURSPACES["ACEScg"].matrix_RGB_to_XYZ[1].tolist(),
        "reference": prepare_reference(profile, negative, responsivities),
        "reference_source": reference_source(responsivities),
        "intent": "The historical Academy universal density reference supplies flux constraints for a minimum-relative-entropy spectrum with an explicit base prior. Adapter-only boundary projection and low-positive receiver continuation retain signed AP1 upstream. The scalar gray tone/tint is calibrated independently; coloured luminance follows the spectral reference and medium response, without a per-colour exposure correction or output hue/saturation fit.",
    }


def expose(inputs, h0, driver, gain=0.6, gray=0.18):
    """Source exposure observations using independent SciPy reference moments."""
    import numpy as np

    luminance = np.array(driver["input_luminance"])
    spectral_reference = Reference(driver["reference"])

    def encode(x):
        return (
            10.5402377416545 * x + 0.0729055341958355
            if x <= 0.0078125
            else (np.log2(x) + 9.72) / 17.52
        )

    def neutral_exposure(brightness):
        return np.array(h0) - gain * 17.52 * (encode(brightness) - encode(gray)) * np.log10(2)

    result = []
    for rgb in np.atleast_2d(inputs):
        if rgb[0] == rgb[1] == rgb[2]:
            result.append(neutral_exposure(float(rgb[0])))
            continue
        brightness = float(luminance @ rgb)
        result.append(neutral_exposure(brightness) + spectral_reference.difference(rgb, brightness))
    return np.array(result)
