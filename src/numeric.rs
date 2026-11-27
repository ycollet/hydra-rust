#[inline]
pub(crate) fn as_number(value: &rhai::Dynamic) -> Option<f64> {
    value
        .as_float()
        .ok()
        .or_else(|| value.as_int().ok().map(|number| number as f64))
}
