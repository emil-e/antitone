"""Reference-negative input and brightness-anchored paper exposure."""

from reference import Reference, prepare as prepare_reference, source as reference_source


def prepare(profile, negative, responsivities):
    """Export the convex spectral selector and original paper photometry."""
    import colour
    import numpy as np
    from spektrafilm.config import STANDARD_OBSERVER_CMFS
    from spektrafilm.model.illuminants import standard_illuminant

    dyes = np.asarray(profile.data.channel_density)
    base = np.asarray(profile.data.base_density)
    support = np.isfinite(dyes).all(axis=1) & np.isfinite(base)
    light = standard_illuminant(profile.info.viewing_illuminant)
    weights = light[:, None] * STANDARD_OBSERVER_CMFS.values
    weights /= weights[:, 1].sum()
    return {
        "input_luminance": colour.RGB_COLOURSPACES["ACEScg"].matrix_RGB_to_XYZ[1].tolist(),
        "photometry": {
            "dyes": dyes[support].tolist(),
            "weights": (weights[support, 1] * 10 ** -base[support]).tolist(),
        },
        "reference": prepare_reference(profile, negative, responsivities),
        "reference_source": reference_source(responsivities),
        "intent": "The historical Academy universal density reference supplies flux constraints for a minimum-relative-entropy spectrum with an explicit base prior. Adapter-only boundary projection and low-positive receiver continuation retain signed AP1 upstream. Original scalar gray tone/tint and common-Y anchoring remain; no output hue/saturation fit or named film characteristic curve.",
    }


def expose(inputs, axis, curves, h0, driver, gain=0.6, gray=0.18):
    """Independent scalar Brent observations for the declared reference input."""
    import numpy as np
    from scipy.optimize import brentq

    dyes = np.array(driver["photometry"]["dyes"])
    weights = np.array(driver["photometry"]["weights"])
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

    def viewed_y(h):
        density = np.array([np.interp(h[i], axis, curves[:, i]) for i in range(3)])
        return float(weights @ np.exp(-np.log(10) * (dyes @ density)))

    result = []
    for rgb in np.atleast_2d(inputs):
        neutral = neutral_exposure(float(luminance @ rgb))
        if rgb[0] == rgb[1] == rgb[2]:
            result.append(neutral_exposure(float(rgb[0])))
            continue
        if np.max(neutral) <= axis[0] or np.min(neutral) >= axis[-1]:
            result.append(neutral)
            continue
        h = neutral + spectral_reference.difference(rgb, float(luminance @ rgb))
        target_y = viewed_y(neutral)
        low = float(axis[0] - np.max(h))
        high = float(axis[-1] - np.min(h))
        offset = brentq(lambda a: viewed_y(h + a) - target_y, low, high, xtol=1e-13)
        result.append(h + offset)
    return np.array(result)
