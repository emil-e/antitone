//! Halation supplies its own finite-neighbour work; common execution owns scratch lifetimes.
use super::*;
use crate::{geometry::SourceRegion as Rect, halation::HalationParameters};

pub(crate) fn halation(
    input: Arc<ImageNode>,
    parameters: HalationParameters,
) -> Result<Arc<ImageNode>, Error> {
    parameters.validate()?;
    if input.domain.meaning != Meaning::Graded {
        return Err(Error::InvalidParameter("halation requires graded AP1"));
    }
    if parameters.strength == 0. {
        return Ok(input);
    }
    for pitch in input.domain.grid.pitch {
        let sigma = parameters.sigma as f32 / pitch as f32;
        if !sigma.is_finite() || sigma <= 0. || (4. * sigma).ceil() as f64 >= f64::from(i32::MAX) {
            return Err(Error::InvalidParameter(
                "halation exceeds GPU representation",
            ));
        }
    }
    Ok(ImageNode::new(
        input.domain,
        Some(input.clone()),
        Halation {
            domain: input.domain,
            parameters,
        },
    ))
}

struct Halation {
    domain: Domain,
    parameters: HalationParameters,
}
impl Operation for Halation {
    fn input(&self, output: Rect) -> Result<Option<Rect>, Error> {
        Ok(Some(
            self.parameters
                .support(self.domain.grid.pitch)
                .expand(output, self.domain.bounds),
        ))
    }

    fn lower(
        &self,
        fragment: &mut Fragment,
        input: Option<PlannedImage>,
        output: PlannedImage,
    ) -> Result<(), Error> {
        let input = input.unwrap();
        let horizontal = fragment.image(input.domain, input.region);
        let sigma = self
            .domain
            .grid
            .pitch
            .map(|pitch| self.parameters.sigma as f32 / pitch as f32);
        let support = self.parameters.support(self.domain.grid.pitch).0;
        let geometry = fragment.uniform(&[
            [
                input.region.width,
                input.region.height,
                output.region.width,
                output.region.height,
            ],
            [
                output.region.x - input.region.x,
                output.region.y - input.region.y,
                support[0],
                support[1],
            ],
            [self.domain.grid.coverage, 0, 0, 0],
        ]);
        let alpha = self.parameters.fractions().map(|v| v as f32);
        let values = fragment.uniform(&[
            [sigma[0], sigma[1], 0., 0.],
            [alpha[0], alpha[1], alpha[2], 0.],
        ]);
        let code: Arc<str> = include_str!("halation.wgsl").into();
        fragment.compute(
            code.clone(),
            "horizontal",
            vec![
                (0, Binding::Read(input.resource)),
                (5, Binding::Write(horizontal.resource)),
                (3, geometry.clone()),
                (4, values.clone()),
            ],
            input.cells(),
        );
        // Keeping input bound here prevents its retirement or reuse before signed recombination.
        fragment.compute(
            code,
            "combine",
            vec![
                (0, Binding::Read(input.resource)),
                (1, Binding::Read(horizontal.resource)),
                (2, Binding::Write(output.resource)),
                (3, geometry),
                (4, values),
            ],
            output.cells(),
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests;
