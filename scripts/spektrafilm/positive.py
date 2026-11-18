"""Fixed per-medium printing of the calibrated paper-only reference negative."""

if __package__:
    from .characteristic import PARAMETERS
    from .reference import Reference
else:
    from characteristic import PARAMETERS
    from reference import Reference


class Driver:
    def __init__(self, film, pair, gray_log_h):
        self.reference = Reference(film, pair)
        gray = self.reference.expose([PARAMETERS["gray"]] * 3)
        self.print_log_h = [aim - actual for aim, actual in zip(gray_log_h, gray)]

    def expose(self, inputs):
        return [
            [
                printing + value
                for printing, value in zip(self.print_log_h, self.reference.expose(rgb))
            ]
            for rgb in inputs
        ]

    def source(self):
        return {
            "negative": self.reference.pair["negative"],
            "paper": self.reference.pair["paper"],
            "print_log_h": self.print_log_h,
        }


def prepare(film, pair, gray_log_h):
    return Driver(film, pair, gray_log_h)


def expose(inputs, driver):
    return driver.expose(inputs)
