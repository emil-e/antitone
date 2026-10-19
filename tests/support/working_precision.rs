/// Current FP32 photographic working comparison budget, independent of the file container.
/// This is not a visibility guarantee. Exact conservation/cancellation/flag witnesses and
/// serialization checks supplement it; quantization is accounted for separately by callers.
pub fn allowance(reference: f64) -> f64 {
    2e-5 + 2e-4 * reference.abs()
}
